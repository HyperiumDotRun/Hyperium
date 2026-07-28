// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {BondingCurveToken} from "./BondingCurveToken.sol";
import {BondingCurve} from "./BondingCurve.sol";

/// @notice Deploys paired (BondingCurveToken, BondingCurve) launches.
/// @dev BondingCurveToken.curve and BondingCurve.token are both immutable, set at
///      construction, and mutually reference each other -- a chicken-and-egg problem
///      with no post-deploy setter available. This factory resolves it by predicting
///      the address its *next* CREATE will land at (the standard EIP-161/RLP
///      deployer+nonce formula) before deploying anything:
///        1. Predict the address the curve will get (this factory's nonce + 1).
///        2. Deploy the token first, wiring in that predicted curve address.
///        3. Deploy the curve, wiring in the token's real (now known) address.
///        4. Assert the curve landed exactly where predicted (defensive sanity check;
///           this would only ever fail if the factory started performing some other
///           CREATE elsewhere, which it does not).
contract BondingCurveFactory {
    event Launched(
        address indexed launcher,
        address indexed token,
        address indexed curve,
        address stockToken,
        uint256 graduationThreshold
    );
    event StockTokenApproved(address indexed stockToken);
    event StockTokenRevoked(address indexed stockToken);
    event FeeBpsChanged(uint256 newFeeBps);
    event FeeRecipientChanged(address indexed newRecipient);
    event LaunchFeeChanged(uint256 newLaunchFee);
    event LaunchFeesWithdrawn(address indexed to, uint256 amount);

    /// @notice Maps both a launched token and its curve to the curve address, so a
    ///         frontend/caller can look either one up interchangeably.
    mapping(address => address) public curveOf;

    /// @notice Curated allowlist of stock tokens a new curve may be paired with --
    ///         without this, `launch` would accept literally any ERC-20 address,
    ///         letting anyone spin up a curve "paired" with a fake token that just
    ///         happens to be named/symboled like a real stock. Only `owner` can
    ///         extend or shrink this list.
    mapping(address => bool) public isApprovedStockToken;

    address public owner;

    /// @notice Trading fee (basis points) baked into every curve launched from now
    ///         on -- changing this only affects future launches, since each curve
    ///         takes an immutable copy of it at construction time.
    uint256 public feeBps;
    address public feeRecipient;
    uint256 public constant MAX_FEE_BPS = 500; // 5% hard cap

    /// @notice Flat native-currency fee required to call `launch` -- the anti-spam
    ///         knob. Zero by default (no barrier) until the owner decides otherwise.
    uint256 public launchFee;

    address[] private _allCurves;

    // Mirrors this contract's own account nonce. EIP-161 gives a freshly deployed
    // contract a starting nonce of 1; every `new` this contract performs (and this
    // contract performs no other CREATE anywhere else) increments it by exactly one,
    // in lockstep with the real EVM account nonce used for CREATE address derivation.
    uint256 private _deployNonce = 1;

    constructor() {
        owner = msg.sender;
        feeRecipient = msg.sender;
    }

    modifier onlyOwner() {
        require(msg.sender == owner, "not owner");
        _;
    }

    function addApprovedStockToken(address stockToken) external onlyOwner {
        require(stockToken != address(0), "zero stock token");
        isApprovedStockToken[stockToken] = true;
        emit StockTokenApproved(stockToken);
    }

    function removeApprovedStockToken(address stockToken) external onlyOwner {
        isApprovedStockToken[stockToken] = false;
        emit StockTokenRevoked(stockToken);
    }

    function setFeeBps(uint256 newFeeBps) external onlyOwner {
        require(newFeeBps <= MAX_FEE_BPS, "fee too high");
        feeBps = newFeeBps;
        emit FeeBpsChanged(newFeeBps);
    }

    function setFeeRecipient(address newRecipient) external onlyOwner {
        require(newRecipient != address(0), "zero recipient");
        feeRecipient = newRecipient;
        emit FeeRecipientChanged(newRecipient);
    }

    function setLaunchFee(uint256 newLaunchFee) external onlyOwner {
        launchFee = newLaunchFee;
        emit LaunchFeeChanged(newLaunchFee);
    }

    /// @notice Sweeps every launch fee collected so far to `to`. Full-balance sweep
    ///         rather than a per-fee ledger: nothing else ever sends this contract
    ///         native currency, so its balance always equals the accrued total.
    function withdrawLaunchFees(address payable to) external onlyOwner {
        require(to != address(0), "zero recipient");
        uint256 amount = address(this).balance;
        emit LaunchFeesWithdrawn(to, amount);
        to.transfer(amount);
    }

    function launch(string calldata name, string calldata symbol, address pairedStockToken, uint256 graduationThreshold)
        external
        payable
        returns (address curveAddress)
    {
        require(msg.value == launchFee, "wrong launch fee");
        require(isApprovedStockToken[pairedStockToken], "stock token not approved");
        require(graduationThreshold > 0, "zero threshold");

        address predictedCurve = _computeCreateAddress(address(this), _deployNonce + 1);

        BondingCurveToken launchedToken = new BondingCurveToken(name, symbol, predictedCurve);
        _deployNonce++;

        BondingCurve curve = new BondingCurve(
            address(launchedToken), pairedStockToken, graduationThreshold, feeBps, feeRecipient
        );
        _deployNonce++;

        require(address(curve) == predictedCurve, "curve address prediction mismatch");

        curveAddress = address(curve);
        curveOf[address(launchedToken)] = curveAddress;
        curveOf[curveAddress] = curveAddress;
        _allCurves.push(curveAddress);

        emit Launched(msg.sender, address(launchedToken), curveAddress, pairedStockToken, graduationThreshold);
    }

    function allCurves() external view returns (address[] memory) {
        return _allCurves;
    }

    function allCurvesCount() external view returns (uint256) {
        return _allCurves.length;
    }

    /// @dev Standard EVM CREATE address derivation: rightmost 20 bytes of
    ///      keccak256(rlp([deployer, nonce])). Handles nonce values up to 2^32-1,
    ///      far beyond anything a realistic factory will ever reach.
    function _computeCreateAddress(address deployer, uint256 nonce) private pure returns (address) {
        bytes memory data;
        if (nonce == 0x00) {
            data = abi.encodePacked(bytes1(0xd6), bytes1(0x94), deployer, bytes1(0x80));
        } else if (nonce <= 0x7f) {
            // forge-lint: disable-next-line(unsafe-typecast) -- guarded by the branch condition above
            data = abi.encodePacked(bytes1(0xd6), bytes1(0x94), deployer, uint8(nonce));
        } else if (nonce <= 0xff) {
            // forge-lint: disable-next-line(unsafe-typecast) -- guarded by the branch condition above
            data = abi.encodePacked(bytes1(0xd7), bytes1(0x94), deployer, bytes1(0x81), uint8(nonce));
        } else if (nonce <= 0xffff) {
            // forge-lint: disable-next-line(unsafe-typecast) -- guarded by the branch condition above
            data = abi.encodePacked(bytes1(0xd8), bytes1(0x94), deployer, bytes1(0x82), uint16(nonce));
        } else if (nonce <= 0xffffff) {
            // forge-lint: disable-next-line(unsafe-typecast) -- guarded by the branch condition above
            data = abi.encodePacked(bytes1(0xd9), bytes1(0x94), deployer, bytes1(0x83), uint24(nonce));
        } else {
            // forge-lint: disable-next-line(unsafe-typecast) -- guarded by the branch condition above
            data = abi.encodePacked(bytes1(0xda), bytes1(0x94), deployer, bytes1(0x84), uint32(nonce));
        }
        return address(uint160(uint256(keccak256(data))));
    }
}
