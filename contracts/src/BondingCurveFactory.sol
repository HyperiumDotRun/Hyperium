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

    /// @notice Maps both a launched token and its curve to the curve address, so a
    ///         frontend/caller can look either one up interchangeably.
    mapping(address => address) public curveOf;

    address[] private _allCurves;

    // Mirrors this contract's own account nonce. EIP-161 gives a freshly deployed
    // contract a starting nonce of 1; every `new` this contract performs (and this
    // contract performs no other CREATE anywhere else) increments it by exactly one,
    // in lockstep with the real EVM account nonce used for CREATE address derivation.
    uint256 private _deployNonce = 1;

    function launch(string calldata name, string calldata symbol, address pairedStockToken, uint256 graduationThreshold)
        external
        returns (address curveAddress)
    {
        require(pairedStockToken != address(0), "zero stock token");
        require(graduationThreshold > 0, "zero threshold");

        address predictedCurve = _computeCreateAddress(address(this), _deployNonce + 1);

        BondingCurveToken launchedToken = new BondingCurveToken(name, symbol, predictedCurve);
        _deployNonce++;

        BondingCurve curve = new BondingCurve(address(launchedToken), pairedStockToken, graduationThreshold);
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
