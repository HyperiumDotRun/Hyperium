// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";

/// @notice ERC-20 launched by a BondingCurveFactory launch. Minting (and burning, for
///         the sell() path) is restricted for the lifetime of the token to a single,
///         immutable "curve" controller address set at construction.
contract BondingCurveToken is ERC20 {
    /// @notice The only address ever allowed to mint or burn this token.
    address public immutable curve;

    error NotCurve();
    error ZeroCurve();

    constructor(string memory name_, string memory symbol_, address curve_) ERC20(name_, symbol_) {
        if (curve_ == address(0)) revert ZeroCurve();
        curve = curve_;
    }

    modifier onlyCurve() {
        if (msg.sender != curve) revert NotCurve();
        _;
    }

    /// @notice Mint new tokens. Only the paired BondingCurve may call this.
    function mint(address to, uint256 amount) external onlyCurve {
        _mint(to, amount);
    }

    /// @notice Burn tokens from an account. Only the paired BondingCurve may call this
    ///         (used by BondingCurve.sell()).
    function burn(address from, uint256 amount) external onlyCurve {
        _burn(from, amount);
    }

    function decimals() public pure override returns (uint8) {
        return 18;
    }
}
