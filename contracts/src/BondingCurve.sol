// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";
import {BondingCurveToken} from "./BondingCurveToken.sol";
import {SimplePool} from "./SimplePool.sol";

/// @notice One instance per launched token. Virtual-reserves constant-product bonding
///         curve (same shape as pump.fun): the launched token is sold for a paired
///         ERC-20 "stock token" (e.g. a testnet TSLA token) rather than for ETH/USDC.
///         Once enough real stock token has been raised, the curve "graduates":
///         it deploys a SimplePool, seeds it with the curve's full real stock-token
///         balance plus a fixed token allocation, permanently locks that initial LP
///         position, and disables further curve trading.
contract BondingCurve is ReentrancyGuard {
    using SafeERC20 for IERC20;

    // ---- Immutable configuration ----
    BondingCurveToken public immutable token;
    IERC20 public immutable stockToken;
    uint256 private immutable _graduationThreshold;

    /// @notice Trading fee taken on every buy/sell, in basis points (1/100 of a
    ///         percent), forwarded immediately to `feeRecipient`. Set once at launch
    ///         time from the factory's current fee settings -- never changes for the
    ///         lifetime of this curve, so a buyer's expected cost can't shift under
    ///         them mid-trade.
    uint256 public immutable feeBps;
    address public immutable feeRecipient;
    uint256 public constant MAX_FEE_BPS = 500; // 5% hard cap, mirrors the factory's own cap

    // ---- Virtual reserves (pump.fun-style constant product bonding curve) ----
    // Hardcoded rather than constructor params so every launch has an identical curve
    // shape (only the paired stock token and graduation threshold vary per launch).
    // 30e18 / 1_073_000_000e18 mirror pump.fun's own well-known constants (30 "quote"
    // units of virtual liquidity against ~1.073B virtual token supply), scaled to
    // 18-decimal stock tokens instead of 9-decimal SOL. This keeps the initial price
    // low but non-zero, rising smoothly as real stock token is deposited.
    uint256 public constant VIRTUAL_STOCK_RESERVE = 30e18;
    uint256 public constant VIRTUAL_TOKEN_RESERVE = 1_073_000_000e18;

    // Fixed token allocation minted at graduation time and locked into the SimplePool
    // as the token-side of initial liquidity, regardless of how many tokens were sold
    // via the curve itself before crossing the graduation threshold.
    uint256 public constant LP_TOKEN_ALLOCATION = 200_000_000e18;

    // ---- State ----
    /// @notice Real stock token raised so far (net of buys/sells), in the stock
    ///         token's raw units. This is what graduationThreshold is compared
    ///         against, and (via stockToken.balanceOf) what actually funds the pool
    ///         at graduation.
    uint256 public raised;
    bool public graduated;
    address public pool;

    event Buy(address indexed buyer, uint256 stockIn, uint256 tokensOut);
    event Sell(address indexed seller, uint256 tokensIn, uint256 stockOut);
    event Fee(address indexed payer, uint256 amount);
    event Graduated(address indexed pool, uint256 stockLiquidity, uint256 tokenLiquidity);

    constructor(
        address token_,
        address stockToken_,
        uint256 graduationThreshold_,
        uint256 feeBps_,
        address feeRecipient_
    ) {
        require(token_ != address(0), "zero token");
        require(stockToken_ != address(0), "zero stock token");
        require(token_ != stockToken_, "identical tokens");
        require(graduationThreshold_ > 0, "zero threshold");
        require(feeBps_ <= MAX_FEE_BPS, "fee too high");
        require(feeBps_ == 0 || feeRecipient_ != address(0), "zero fee recipient");
        token = BondingCurveToken(token_);
        stockToken = IERC20(stockToken_);
        _graduationThreshold = graduationThreshold_;
        feeBps = feeBps_;
        feeRecipient = feeRecipient_;
    }

    function graduationThreshold() external view returns (uint256) {
        return _graduationThreshold;
    }

    /// @notice Preview-only quote for buy(). Uses the exact same internal helper as
    ///         buy() itself, so an external caller can trust previewBuy() == what
    ///         buy() will actually execute (module any state changes in between).
    ///         Quoted on the net-of-fee amount -- see buy()'s comment for why.
    function previewBuy(uint256 stockIn) external view returns (uint256) {
        return _quoteBuy(stockIn - (stockIn * feeBps) / 10_000);
    }

    /// @notice Preview-only quote for sell(). See previewBuy() notes.
    function previewSell(uint256 tokensIn) external view returns (uint256) {
        return _quoteSell(tokensIn);
    }

    function buy(uint256 stockIn, uint256 minTokensOut) external nonReentrant {
        require(!graduated, "graduated: trade on the pool instead");
        require(stockIn > 0, "zero input");

        // Priced on the net-of-fee amount -- that's what actually joins the curve's
        // reserve, and pricing must match the reserve update exactly or a later
        // sell() could demand more real stock token than the curve actually holds.
        uint256 fee = (stockIn * feeBps) / 10_000;
        uint256 netIn = stockIn - fee;
        uint256 tokensOut = _quoteBuy(netIn);
        require(tokensOut >= minTokensOut, "slippage: insufficient output");

        stockToken.safeTransferFrom(msg.sender, address(this), stockIn);

        raised += netIn;
        if (fee > 0) {
            stockToken.safeTransfer(feeRecipient, fee);
            emit Fee(msg.sender, fee);
        }
        token.mint(msg.sender, tokensOut);

        emit Buy(msg.sender, stockIn, tokensOut);

        // Graduation is only ever triggered from here, while this function still
        // holds the nonReentrant lock above -- so the graduation path is covered by
        // the same guard without needing (and being unable to safely re-enter) its
        // own nonReentrant modifier.
        if (!graduated && raised >= _graduationThreshold) {
            _graduate();
        }
    }

    function sell(uint256 tokensIn, uint256 minStockOut) external nonReentrant {
        require(!graduated, "graduated: trade on the pool instead");
        require(tokensIn > 0, "zero input");

        uint256 stockOut = _quoteSell(tokensIn);
        require(stockOut <= raised, "insufficient real stock liquidity");
        require(stockToken.balanceOf(address(this)) >= stockOut, "insufficient real stock liquidity");

        uint256 fee = (stockOut * feeBps) / 10_000;
        uint256 netOut = stockOut - fee;
        // Slippage is checked against what the seller actually receives, not the
        // pre-fee quote -- that's the number they can actually set an expectation on.
        require(netOut >= minStockOut, "slippage: insufficient output");

        token.burn(msg.sender, tokensIn);
        raised -= stockOut;
        if (fee > 0) {
            stockToken.safeTransfer(feeRecipient, fee);
            emit Fee(msg.sender, fee);
        }
        stockToken.safeTransfer(msg.sender, netOut);

        emit Sell(msg.sender, tokensIn, stockOut);
    }

    // ---- Shared pricing math -----------------------------------------------------
    // _quoteBuy / _quoteSell are the ONLY place the constant-product formula is
    // computed. previewBuy/previewSell and buy/sell all call into these, so pricing
    // can never drift between the "preview" and "execute" paths.

    function _reserves() private view returns (uint256 stockReserve, uint256 tokenReserve) {
        stockReserve = VIRTUAL_STOCK_RESERVE + raised;
        tokenReserve = VIRTUAL_TOKEN_RESERVE - token.totalSupply();
    }

    // NOTE: the output amount is floored directly (rounding in the curve's favor),
    // rather than computed by flooring a *new reserve* and subtracting -- the latter
    // is algebraically equivalent to *ceiling* the output, which is the wrong
    // direction for safety and can let a buy-then-sell-the-same-output round trip
    // return 1 wei more than was paid in. Flooring the output directly guarantees
    // (provably, not just empirically) that selling back everything just bought via
    // _quoteBuy can never return more stock token than was spent.
    function _quoteBuy(uint256 stockIn) private view returns (uint256 tokensOut) {
        (uint256 stockReserve, uint256 tokenReserve) = _reserves();
        tokensOut = (stockIn * tokenReserve) / (stockReserve + stockIn);
    }

    function _quoteSell(uint256 tokensIn) private view returns (uint256 stockOut) {
        (uint256 stockReserve, uint256 tokenReserve) = _reserves();
        stockOut = (tokensIn * stockReserve) / (tokenReserve + tokensIn);
    }

    // ---- Graduation ----------------------------------------------------------------

    function _graduate() private {
        graduated = true;

        uint256 stockLiquidity = stockToken.balanceOf(address(this));

        // Mint the fixed LP token allocation to ourselves so it can be transferred
        // into the new pool alongside the real stock token balance.
        token.mint(address(this), LP_TOKEN_ALLOCATION);

        SimplePool newPool = new SimplePool(address(token), address(stockToken));

        IERC20(address(token)).safeTransfer(address(newPool), LP_TOKEN_ALLOCATION);
        stockToken.safeTransfer(address(newPool), stockLiquidity);

        // Mint the initial LP position directly to address(0): permanently and
        // truly unspendable, not merely sent-and-forgettable.
        newPool.mint(address(0));

        pool = address(newPool);

        emit Graduated(address(newPool), stockLiquidity, LP_TOKEN_ALLOCATION);
    }
}
