// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {Vm} from "forge-std/Vm.sol";
import {BondingCurveFactory} from "../src/BondingCurveFactory.sol";
import {BondingCurve} from "../src/BondingCurve.sol";
import {BondingCurveToken} from "../src/BondingCurveToken.sol";
import {MockERC20} from "./mocks/MockERC20.sol";

contract BondingCurveFactoryTest is Test {
    BondingCurveFactory internal factory;
    MockERC20 internal stockA;
    MockERC20 internal stockB;

    address internal alice = makeAddr("alice");

    function setUp() public {
        factory = new BondingCurveFactory();
        stockA = new MockERC20("Testnet TSLA", "TSLAon");
        stockB = new MockERC20("Testnet AAPL", "AAPLon");
    }

    function test_Launch_ReturnsWorkingIndependentCurves_SameStockToken() public {
        address curveAddr1 = factory.launch("Moon Token", "MOON", address(stockA), 50e18);
        address curveAddr2 = factory.launch("Rocket Token", "RKT", address(stockA), 75e18);

        assertTrue(curveAddr1 != curveAddr2);

        BondingCurve curve1 = BondingCurve(curveAddr1);
        BondingCurve curve2 = BondingCurve(curveAddr2);

        assertTrue(address(curve1.token()) != address(curve2.token()));
        assertEq(curve1.graduationThreshold(), 50e18);
        assertEq(curve2.graduationThreshold(), 75e18);

        stockA.mint(alice, 1_000e18);
        vm.startPrank(alice);
        stockA.approve(curveAddr1, type(uint256).max);
        stockA.approve(curveAddr2, type(uint256).max);

        curve1.buy(10e18, 0);
        vm.stopPrank();

        // Buying on curve1 must not affect curve2's state at all.
        assertGt(curve1.raised(), 0);
        assertEq(curve2.raised(), 0);
    }

    function test_Launch_WorksAgainstDifferentStockTokens() public {
        address curveAddr1 = factory.launch("Moon Token", "MOON", address(stockA), 50e18);
        address curveAddr2 = factory.launch("Apple Token", "APLT", address(stockB), 50e18);

        BondingCurve curve1 = BondingCurve(curveAddr1);
        BondingCurve curve2 = BondingCurve(curveAddr2);

        assertEq(address(curve1.stockToken()), address(stockA));
        assertEq(address(curve2.stockToken()), address(stockB));

        stockA.mint(alice, 1_000e18);
        stockB.mint(alice, 1_000e18);
        vm.startPrank(alice);
        stockA.approve(curveAddr1, type(uint256).max);
        stockB.approve(curveAddr2, type(uint256).max);

        curve1.buy(5e18, 0);
        curve2.buy(7e18, 0);
        vm.stopPrank();

        assertEq(curve1.raised(), 5e18);
        assertEq(curve2.raised(), 7e18);
    }

    function test_CurveOf_LooksUpBothTokenAndCurve() public {
        address curveAddr = factory.launch("Moon Token", "MOON", address(stockA), 50e18);
        BondingCurve curve = BondingCurve(curveAddr);
        address tokenAddr = address(curve.token());

        assertEq(factory.curveOf(tokenAddr), curveAddr, "token address must resolve to its curve");
        assertEq(factory.curveOf(curveAddr), curveAddr, "curve address must resolve to itself");
        assertEq(factory.curveOf(address(0xdead)), address(0), "unrelated address must resolve to nothing");
    }

    function test_AllCurves_EnumeratesEveryLaunch() public {
        address c1 = factory.launch("A", "AAA", address(stockA), 10e18);
        address c2 = factory.launch("B", "BBB", address(stockA), 20e18);
        address c3 = factory.launch("C", "CCC", address(stockB), 30e18);

        address[] memory all = factory.allCurves();
        assertEq(all.length, 3);
        assertEq(all[0], c1);
        assertEq(all[1], c2);
        assertEq(all[2], c3);
        assertEq(factory.allCurvesCount(), 3);
    }

    function test_Launch_EmitsLaunchedEvent() public {
        // We don't know the token/curve addresses ahead of time, so only check the
        // non-address topic (launcher) and the data payload precisely via manual replay.
        vm.recordLogs();
        address curveAddr = factory.launch("Moon Token", "MOON", address(stockA), 50e18);
        BondingCurve curve = BondingCurve(curveAddr);

        Vm.Log[] memory entries = vm.getRecordedLogs();
        bool found;
        for (uint256 i = 0; i < entries.length; i++) {
            if (entries[i].topics[0] == keccak256("Launched(address,address,address,address,uint256)")) {
                found = true;
                assertEq(address(uint160(uint256(entries[i].topics[1]))), address(this));
                assertEq(address(uint160(uint256(entries[i].topics[2]))), address(curve.token()));
                assertEq(address(uint160(uint256(entries[i].topics[3]))), curveAddr);
                (address stockTokenArg, uint256 thresholdArg) = abi.decode(entries[i].data, (address, uint256));
                assertEq(stockTokenArg, address(stockA));
                assertEq(thresholdArg, 50e18);
            }
        }
        assertTrue(found, "Launched event must be emitted");
    }
}
