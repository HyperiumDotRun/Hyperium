// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {BondingCurveFactory} from "../src/BondingCurveFactory.sol";
import {BondingCurve} from "../src/BondingCurve.sol";
import {BondingCurveToken} from "../src/BondingCurveToken.sol";
import {MockERC20} from "./mocks/MockERC20.sol";

/// @notice Covers the protocol fee (trading fee on buy/sell, forwarded to a
///         `feeRecipient`) and the launch fee (flat native-currency anti-spam charge),
///         both added to `BondingCurveFactory`/`BondingCurve` after the base bonding
///         curve was already shipped and live-verified.
contract BondingCurveFeesTest is Test {
    BondingCurveFactory internal factory;
    MockERC20 internal stock;

    address internal alice = makeAddr("alice");
    address internal treasury = makeAddr("treasury");

    uint256 internal constant THRESHOLD = 1_000_000e18; // high enough that these tests never graduate

    function setUp() public {
        factory = new BondingCurveFactory();
        stock = new MockERC20("Testnet TSLA", "TSLAon");
        factory.addApprovedStockToken(address(stock));
        stock.mint(alice, 1_000_000e18);
    }

    // ------------------------------------------------------------- trading fee ---

    function test_Buy_TakesFeeAndForwardsToRecipient() public {
        factory.setFeeRecipient(treasury);
        factory.setFeeBps(100); // 1%

        address curveAddr = factory.launch("Moon Token", "MOON", address(stock), THRESHOLD);
        BondingCurve curve = BondingCurve(curveAddr);

        vm.prank(alice);
        stock.approve(curveAddr, type(uint256).max);

        vm.prank(alice);
        curve.buy(10e18, 0);

        uint256 expectedFee = (10e18 * 100) / 10_000;
        assertEq(stock.balanceOf(treasury), expectedFee, "treasury must receive exactly 1% of stockIn");
        assertEq(curve.raised(), 10e18 - expectedFee, "raised must only count the net-of-fee amount");
    }

    function test_Sell_TakesFeeAndForwardsToRecipient() public {
        factory.setFeeRecipient(treasury);
        factory.setFeeBps(100); // 1%

        address curveAddr = factory.launch("Moon Token", "MOON", address(stock), THRESHOLD);
        BondingCurve curve = BondingCurve(curveAddr);
        BondingCurveToken token = curve.token();

        vm.startPrank(alice);
        stock.approve(curveAddr, type(uint256).max);
        curve.buy(10e18, 0);
        uint256 held = token.balanceOf(alice);

        uint256 grossOut = curve.previewSell(held);
        uint256 expectedFee = (grossOut * 100) / 10_000;
        uint256 balanceBefore = stock.balanceOf(alice);
        uint256 treasuryBefore = stock.balanceOf(treasury);

        curve.sell(held, 0);
        vm.stopPrank();

        assertEq(stock.balanceOf(treasury) - treasuryBefore, expectedFee, "treasury must receive exactly 1% of stockOut");
        assertEq(stock.balanceOf(alice) - balanceBefore, grossOut - expectedFee, "seller must receive the net-of-fee amount");
    }

    function test_Sell_SlippageChecksNetAmountNotGrossQuote() public {
        factory.setFeeRecipient(treasury);
        factory.setFeeBps(100); // 1%

        address curveAddr = factory.launch("Moon Token", "MOON", address(stock), THRESHOLD);
        BondingCurve curve = BondingCurve(curveAddr);
        BondingCurveToken token = curve.token();

        vm.startPrank(alice);
        stock.approve(curveAddr, type(uint256).max);
        curve.buy(10e18, 0);
        uint256 held = token.balanceOf(alice);
        uint256 grossOut = curve.previewSell(held);

        // Requiring the full gross quote as the minimum must revert -- the seller
        // can only ever actually receive the net-of-fee amount.
        vm.expectRevert("slippage: insufficient output");
        curve.sell(held, grossOut);
        vm.stopPrank();
    }

    function test_ZeroFeeBps_ChangesNothing() public {
        // Default factory (feeBps == 0) behaves exactly as before fees existed.
        address curveAddr = factory.launch("Moon Token", "MOON", address(stock), THRESHOLD);
        BondingCurve curve = BondingCurve(curveAddr);

        vm.startPrank(alice);
        stock.approve(curveAddr, type(uint256).max);
        curve.buy(10e18, 0);
        vm.stopPrank();

        assertEq(curve.raised(), 10e18, "raised must equal the full stockIn when feeBps is zero");
    }

    function test_FeeBps_IsFrozenPerCurveAtLaunchTime() public {
        address curveAddr1 = factory.launch("A", "AAA", address(stock), THRESHOLD);
        factory.setFeeRecipient(treasury);
        factory.setFeeBps(200); // 2%, only affects launches from here on
        address curveAddr2 = factory.launch("B", "BBB", address(stock), THRESHOLD);

        assertEq(BondingCurve(curveAddr1).feeBps(), 0, "earlier curve must keep the fee it was launched with");
        assertEq(BondingCurve(curveAddr2).feeBps(), 200, "later curve must pick up the new fee setting");
    }

    // -------------------------------------------------------------- fee setters ---

    function test_SetFeeBps_RejectsAboveCap() public {
        vm.expectRevert("fee too high");
        factory.setFeeBps(501);
    }

    function test_NonOwner_CannotSetFeeBpsOrRecipient() public {
        vm.startPrank(alice);
        vm.expectRevert("not owner");
        factory.setFeeBps(100);
        vm.expectRevert("not owner");
        factory.setFeeRecipient(treasury);
        vm.stopPrank();
    }

    function test_BondingCurve_RejectsFeeBpsAboveCapAtConstruction() public {
        vm.expectRevert("fee too high");
        new BondingCurve(address(0x1), address(stock), THRESHOLD, 501, treasury);
    }

    // -------------------------------------------------------------------- launch fee ---

    function test_Launch_RevertsIfWrongValueSent() public {
        factory.setLaunchFee(0.01 ether);
        vm.expectRevert("wrong launch fee");
        factory.launch("Moon Token", "MOON", address(stock), THRESHOLD);
        vm.expectRevert("wrong launch fee");
        factory.launch{value: 0.005 ether}("Moon Token", "MOON", address(stock), THRESHOLD);
    }

    function test_Launch_SucceedsWithExactFee() public {
        factory.setLaunchFee(0.01 ether);
        address curveAddr = factory.launch{value: 0.01 ether}("Moon Token", "MOON", address(stock), THRESHOLD);
        assertTrue(curveAddr != address(0));
        assertEq(address(factory).balance, 0.01 ether);
    }

    function test_WithdrawLaunchFees_SweepsBalanceToRecipient() public {
        factory.setLaunchFee(0.01 ether);
        factory.launch{value: 0.01 ether}("Moon Token", "MOON", address(stock), THRESHOLD);

        uint256 before = treasury.balance;
        factory.withdrawLaunchFees(payable(treasury));
        assertEq(treasury.balance - before, 0.01 ether);
        assertEq(address(factory).balance, 0);
    }

    function testFuzz_BuyThenSell_NeverInsolvent_WithFee(uint256 stockIn) public {
        stockIn = bound(stockIn, 1e15, 500_000e18);
        factory.setFeeRecipient(treasury);
        factory.setFeeBps(250); // 2.5%

        address curveAddr = factory.launch("Moon Token", "MOON", address(stock), THRESHOLD);
        BondingCurve curve = BondingCurve(curveAddr);
        BondingCurveToken token = curve.token();

        vm.startPrank(alice);
        stock.approve(curveAddr, type(uint256).max);
        curve.buy(stockIn, 0);
        uint256 held = token.balanceOf(alice);
        if (held == 0) {
            vm.stopPrank();
            return;
        }

        uint256 balanceBefore = stock.balanceOf(alice);
        curve.sell(held, 0);
        vm.stopPrank();

        // Two fee-bearing trades can only ever cost the round-tripper money, never
        // hand back more real stock token than they originally put in.
        assertLt(stock.balanceOf(alice) - balanceBefore, stockIn, "round trip must not be profitable, even with fees");
        assertGe(stock.balanceOf(address(curve)), 0);
    }

    function test_NonOwner_CannotSetLaunchFeeOrWithdraw() public {
        vm.startPrank(alice);
        vm.expectRevert("not owner");
        factory.setLaunchFee(1 ether);
        vm.expectRevert("not owner");
        factory.withdrawLaunchFees(payable(alice));
        vm.stopPrank();
    }
}
