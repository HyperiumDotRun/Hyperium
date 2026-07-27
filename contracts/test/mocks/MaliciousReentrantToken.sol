// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {ERC20} from "@openzeppelin/contracts/token/ERC20/ERC20.sol";

interface IAttackable {
    function buy(uint256 stockIn, uint256 minTokensOut) external;
    function sell(uint256 tokensIn, uint256 minStockOut) external;
}

/// @notice A non-standard, malicious ERC-20 whose transfer/transferFrom hooks try to
///         re-enter a target BondingCurve mid-transfer. A plain, spec-compliant
///         ERC-20 (as required for the real paired stock token) has no such hook and
///         cannot do this -- this mock exists purely to prove BondingCurve's
///         ReentrancyGuard actually blocks it if a non-standard token ever tried.
contract MaliciousReentrantToken is ERC20 {
    address public target;
    bool public attackOnTransferFrom;
    bool public attackOnTransfer;

    constructor() ERC20("Malicious", "EVIL") {}

    function mint(address to, uint256 amount) external {
        _mint(to, amount);
    }

    function setTarget(address target_) external {
        target = target_;
    }

    function setAttack(bool onTransferFrom, bool onTransfer) external {
        attackOnTransferFrom = onTransferFrom;
        attackOnTransfer = onTransfer;
    }

    function transferFrom(address from, address to, uint256 amount) public override returns (bool) {
        if (attackOnTransferFrom && target != address(0)) {
            IAttackable(target).buy(1, 0);
        }
        return super.transferFrom(from, to, amount);
    }

    function transfer(address to, uint256 amount) public override returns (bool) {
        if (attackOnTransfer && target != address(0)) {
            IAttackable(target).sell(1, 0);
        }
        return super.transfer(to, amount);
    }
}
