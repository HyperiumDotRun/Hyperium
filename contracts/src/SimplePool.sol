// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {IERC20} from "@openzeppelin/contracts/token/ERC20/IERC20.sol";
import {SafeERC20} from "@openzeppelin/contracts/token/ERC20/utils/SafeERC20.sol";
import {ReentrancyGuard} from "@openzeppelin/contracts/utils/ReentrancyGuard.sol";

/// @notice Minimal, self-contained Uniswap-V2-style constant product pair.
/// @dev Used only as the graduation target for BondingCurve in this testnet PoC.
///      Single 0.3% fee tier, no flash-swap callback, no protocol fee switch, and
///      (unlike real Uniswap V2) reserves are plain uint256 rather than packed
///      uint112 + a block-timestamp price accumulator, since no TWAP oracle is
///      needed here. That means the swap() K-invariant check multiplies larger
///      numbers than mainnet Uniswap V2 does; for pathologically large reserves it
///      could overflow and revert (fails safe) rather than silently wrap, which is
///      an acceptable simplification for a testnet PoC.
///
///      The LP share token is intentionally NOT OpenZeppelin's ERC20: OZ's
///      implementation reverts on any mint/transfer whose recipient is address(0)
///      (ERC20InvalidReceiver), but BondingCurve's graduation flow needs to mint the
///      permanently-locked initial LP position directly to address(0). Real Uniswap
///      V2's Pair contract has exactly the same requirement (it burns MINIMUM_LIQUIDITY
///      to address(0) on first mint) and solves it the same way: by inlining its own
///      minimal ERC20 bookkeeping instead of relying on a library that guards against
///      zero-address crediting.
contract SimplePool is ReentrancyGuard {
    using SafeERC20 for IERC20;

    // ---------------------------------------------------------------- LP token ---
    string public constant name = "SimplePool LP";
    string public constant symbol = "SP-LP";
    uint8 public constant decimals = 18;

    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        emit Approval(msg.sender, spender, amount);
        return true;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        _transfer(msg.sender, to, amount);
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        uint256 allowed = allowance[from][msg.sender];
        if (allowed != type(uint256).max) {
            require(allowed >= amount, "insufficient allowance");
            allowance[from][msg.sender] = allowed - amount;
        }
        _transfer(from, to, amount);
        return true;
    }

    function _transfer(address from, address to, uint256 amount) private {
        require(to != address(0), "transfer to zero address");
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        emit Transfer(from, to, amount);
    }

    /// @dev Unlike a public transfer, minting deliberately allows `to == address(0)`
    ///      -- that is precisely how BondingCurve permanently locks the initial LP
    ///      position at graduation.
    function _mint(address to, uint256 amount) private {
        totalSupply += amount;
        balanceOf[to] += amount;
        emit Transfer(address(0), to, amount);
    }

    function _burn(address from, uint256 amount) private {
        balanceOf[from] -= amount;
        totalSupply -= amount;
        emit Transfer(from, address(0), amount);
    }

    // ------------------------------------------------------------------- pool ----

    uint256 public constant MINIMUM_LIQUIDITY = 1000;

    IERC20 public immutable token0;
    IERC20 public immutable token1;

    uint256 public reserve0;
    uint256 public reserve1;

    event Mint(address indexed sender, address indexed to, uint256 amount0, uint256 amount1, uint256 liquidity);
    event Burn(address indexed sender, address indexed to, uint256 amount0, uint256 amount1, uint256 liquidity);
    event Swap(
        address indexed sender,
        address indexed to,
        uint256 amount0In,
        uint256 amount1In,
        uint256 amount0Out,
        uint256 amount1Out
    );

    constructor(address token0_, address token1_) {
        require(token0_ != address(0) && token1_ != address(0), "zero token");
        require(token0_ != token1_, "identical tokens");
        token0 = IERC20(token0_);
        token1 = IERC20(token1_);
    }

    function getReserves() external view returns (uint256, uint256) {
        return (reserve0, reserve1);
    }

    /// @notice Mint LP tokens for whatever token0/token1 have already been
    ///         transferred into this contract beyond the last recorded reserves.
    ///         Standard "transfer then call" Uniswap-V2 pattern.
    function mint(address to) external nonReentrant returns (uint256 liquidity) {
        uint256 balance0 = token0.balanceOf(address(this));
        uint256 balance1 = token1.balanceOf(address(this));
        uint256 amount0 = balance0 - reserve0;
        uint256 amount1 = balance1 - reserve1;

        uint256 totalSupply_ = totalSupply;
        if (totalSupply_ == 0) {
            liquidity = _sqrt(amount0 * amount1) - MINIMUM_LIQUIDITY;
            _mint(address(0), MINIMUM_LIQUIDITY);
        } else {
            liquidity = _min((amount0 * totalSupply_) / reserve0, (amount1 * totalSupply_) / reserve1);
        }
        require(liquidity > 0, "insufficient liquidity minted");
        _mint(to, liquidity);

        reserve0 = balance0;
        reserve1 = balance1;
        emit Mint(msg.sender, to, amount0, amount1, liquidity);
    }

    /// @notice Burn LP tokens already transferred into this contract, sending the
    ///         underlying token0/token1 share to `to`. Standard "transfer LP then
    ///         call" Uniswap-V2 pattern.
    function burn(address to) external nonReentrant returns (uint256 amount0, uint256 amount1) {
        uint256 balance0 = token0.balanceOf(address(this));
        uint256 balance1 = token1.balanceOf(address(this));
        uint256 liquidity = balanceOf[address(this)];

        uint256 totalSupply_ = totalSupply;
        amount0 = (liquidity * balance0) / totalSupply_;
        amount1 = (liquidity * balance1) / totalSupply_;
        require(amount0 > 0 && amount1 > 0, "insufficient liquidity burned");

        _burn(address(this), liquidity);
        token0.safeTransfer(to, amount0);
        token1.safeTransfer(to, amount1);

        reserve0 = token0.balanceOf(address(this));
        reserve1 = token1.balanceOf(address(this));
        emit Burn(msg.sender, to, amount0, amount1, liquidity);
    }

    /// @notice Low-level swap: caller must have already transferred the input
    ///         token(s) into this contract before calling, and specifies the
    ///         desired output amount(s). Reverts if the post-swap constant-product
    ///         invariant (net of the 0.3% fee) does not hold.
    function swap(uint256 amount0Out, uint256 amount1Out, address to) external nonReentrant {
        require(amount0Out > 0 || amount1Out > 0, "insufficient output amount");
        require(amount0Out < reserve0 && amount1Out < reserve1, "insufficient liquidity");
        require(to != address(token0) && to != address(token1), "invalid to");

        if (amount0Out > 0) token0.safeTransfer(to, amount0Out);
        if (amount1Out > 0) token1.safeTransfer(to, amount1Out);

        uint256 balance0 = token0.balanceOf(address(this));
        uint256 balance1 = token1.balanceOf(address(this));

        uint256 amount0In = balance0 + amount0Out > reserve0 ? balance0 + amount0Out - reserve0 : 0;
        uint256 amount1In = balance1 + amount1Out > reserve1 ? balance1 + amount1Out - reserve1 : 0;
        require(amount0In > 0 || amount1In > 0, "insufficient input amount");

        uint256 balance0Adjusted = balance0 * 1000 - amount0In * 3;
        uint256 balance1Adjusted = balance1 * 1000 - amount1In * 3;
        require(balance0Adjusted * balance1Adjusted >= reserve0 * reserve1 * 1_000_000, "K invariant");

        reserve0 = balance0;
        reserve1 = balance1;
        emit Swap(msg.sender, to, amount0In, amount1In, amount0Out, amount1Out);
    }

    function _sqrt(uint256 y) internal pure returns (uint256 z) {
        if (y > 3) {
            z = y;
            uint256 x = y / 2 + 1;
            while (x < z) {
                z = x;
                x = (y / x + x) / 2;
            }
        } else if (y != 0) {
            z = 1;
        }
    }

    function _min(uint256 a, uint256 b) internal pure returns (uint256) {
        return a < b ? a : b;
    }
}
