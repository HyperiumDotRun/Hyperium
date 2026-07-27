// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {BondingCurveFactory} from "../src/BondingCurveFactory.sol";
import {BondingCurve} from "../src/BondingCurve.sol";
import {BondingCurveToken} from "../src/BondingCurveToken.sol";
import {SimplePool} from "../src/SimplePool.sol";
import {MockERC20} from "./mocks/MockERC20.sol";
import {MaliciousReentrantToken} from "./mocks/MaliciousReentrantToken.sol";

contract BondingCurveTest is Test {
    BondingCurveFactory internal factory;
    MockERC20 internal stock;
    BondingCurve internal curve;
    BondingCurveToken internal token;

    address internal alice = makeAddr("alice");
    address internal bob = makeAddr("bob");

    uint256 internal constant THRESHOLD = 50e18;

    function setUp() public {
        factory = new BondingCurveFactory();
        stock = new MockERC20("Testnet TSLA", "TSLAon");

        address curveAddr = factory.launch("Moon Token", "MOON", address(stock), THRESHOLD);
        curve = BondingCurve(curveAddr);
        token = curve.token();

        stock.mint(alice, 1_000_000e18);
        stock.mint(bob, 1_000_000e18);

        vm.prank(alice);
        stock.approve(curveAddr, type(uint256).max);
        vm.prank(bob);
        stock.approve(curveAddr, type(uint256).max);
    }

    // ------------------------------------------------------------------ pricing ---

    function test_PreviewBuyMatchesActual_AcrossSeveralSizes() public {
        uint256[4] memory sizes = [uint256(0.1e18), 1e18, 3e18, 7e18];
        for (uint256 i = 0; i < sizes.length; i++) {
            uint256 preview = curve.previewBuy(sizes[i]);
            uint256 before = token.balanceOf(alice);

            vm.prank(alice);
            curve.buy(sizes[i], 0);

            uint256 gained = token.balanceOf(alice) - before;
            assertEq(gained, preview, "previewBuy must equal actual buy() output");
        }
    }

    function test_PreviewSellMatchesActual_AcrossSeveralSizes() public {
        vm.prank(alice);
        curve.buy(20e18, 0);
        uint256 held = token.balanceOf(alice);

        uint256[3] memory fractions = [held / 10, held / 5, held / 20];
        for (uint256 i = 0; i < fractions.length; i++) {
            if (fractions[i] == 0) continue;
            uint256 preview = curve.previewSell(fractions[i]);
            uint256 before = stock.balanceOf(alice);

            vm.prank(alice);
            curve.sell(fractions[i], 0);

            uint256 gained = stock.balanceOf(alice) - before;
            assertEq(gained, preview, "previewSell must equal actual sell() output");
        }
    }

    function test_PriceRisesMonotonically_AsMoreIsBought() public {
        uint256 fixedStockIn = 1e18;
        uint256 previousTokensOut = type(uint256).max;

        for (uint256 i = 0; i < 6; i++) {
            uint256 tokensOut = curve.previewBuy(fixedStockIn);
            assertLt(tokensOut, previousTokensOut, "equal stock input must buy strictly fewer tokens as price rises");
            previousTokensOut = tokensOut;

            vm.prank(alice);
            curve.buy(fixedStockIn, 0);
        }
    }

    function test_RoundTrip_BuyThenSell_NoFreeProfit() public {
        uint256 stockIn = 10e18;

        vm.startPrank(alice);
        uint256 tokensOut = curve.previewBuy(stockIn);
        curve.buy(stockIn, 0);

        uint256 stockBack = curve.previewSell(tokensOut);
        curve.sell(tokensOut, 0);
        vm.stopPrank();

        assertLe(stockBack, stockIn, "round-tripping the exact same size must never profit");
        // Only rounding should be lost, not a meaningful amount.
        assertApproxEqAbs(stockBack, stockIn, 2, "round-trip loss should be rounding-sized only");
    }

    function test_PreviewIsPureAndDoesNotChangeState() public view {
        uint256 raisedBefore = curve.raised();
        curve.previewBuy(5e18);
        curve.previewSell(1e18);
        assertEq(curve.raised(), raisedBefore, "preview must not mutate state");
    }

    // --------------------------------------------------------------- graduation ---

    function test_Graduation_FiresWhenCrossingThreshold_SingleBigBuy() public {
        assertFalse(curve.graduated());
        assertEq(curve.pool(), address(0));

        // Comfortably larger than THRESHOLD so this single buy jumps straight past it.
        vm.prank(alice);
        curve.buy(THRESHOLD * 3, 0);

        assertTrue(curve.graduated());
        assertGe(curve.raised(), THRESHOLD);
        assertTrue(curve.pool() != address(0));
    }

    function test_Graduation_FiresExactlyOnce_AcrossMultipleBuys() public {
        vm.prank(alice);
        curve.buy(THRESHOLD / 2, 0);
        assertFalse(curve.graduated());

        vm.prank(alice);
        curve.buy(THRESHOLD, 0); // this one crosses the threshold
        assertTrue(curve.graduated());
        address poolAfterFirstGraduation = curve.pool();
        assertTrue(poolAfterFirstGraduation != address(0));

        // Any further trade must revert -- graduation cannot be re-triggered, and the
        // pool address must never change afterwards.
        vm.prank(bob);
        vm.expectRevert(bytes("graduated: trade on the pool instead"));
        curve.buy(1e18, 0);

        assertEq(curve.pool(), poolAfterFirstGraduation);
    }

    function test_Graduation_BuyAndSellRevertAfterwards() public {
        vm.prank(alice);
        curve.buy(THRESHOLD * 2, 0);
        assertTrue(curve.graduated());

        vm.prank(bob);
        vm.expectRevert(bytes("graduated: trade on the pool instead"));
        curve.buy(1e18, 0);

        vm.prank(alice);
        vm.expectRevert(bytes("graduated: trade on the pool instead"));
        curve.sell(1, 0);
    }

    function test_Graduation_PoolHoldsExpectedLockedLiquidity() public {
        vm.prank(alice);
        curve.buy(THRESHOLD * 2, 0);
        assertTrue(curve.graduated());

        SimplePool pool = SimplePool(curve.pool());

        // Pool must hold exactly the curve's real stock-token raise and the fixed
        // LP token allocation.
        assertEq(stock.balanceOf(address(pool)), curve.raised(), "pool must hold the curve's full real stock balance");
        assertEq(token.balanceOf(address(pool)), curve.LP_TOKEN_ALLOCATION(), "pool must hold the fixed LP token allocation");

        // The ENTIRE initial LP supply must be held by address(0) -- truly locked,
        // not just partially.
        assertEq(pool.balanceOf(address(0)), pool.totalSupply(), "all initial LP supply must be locked at address(0)");
        assertGt(pool.totalSupply(), 0);
    }

    // --------------------------------------------------------------- reentrancy ---

    function test_Reentrancy_BuyBlocked() public {
        MaliciousReentrantToken evil = new MaliciousReentrantToken();
        address curveAddr = factory.launch("Evil Token", "EVILT", address(evil), THRESHOLD);
        BondingCurve evilCurve = BondingCurve(curveAddr);

        evil.mint(alice, 1_000e18);
        vm.prank(alice);
        evil.approve(curveAddr, type(uint256).max);

        evil.setTarget(curveAddr);
        evil.setAttack(true, false); // attack on transferFrom, which buy() triggers

        vm.prank(alice);
        vm.expectRevert();
        evilCurve.buy(10e18, 0);
    }

    function test_Reentrancy_SellBlocked() public {
        MaliciousReentrantToken evil = new MaliciousReentrantToken();
        address curveAddr = factory.launch("Evil Token", "EVILT", address(evil), THRESHOLD);
        BondingCurve evilCurve = BondingCurve(curveAddr);
        BondingCurveToken evilToken = evilCurve.token();

        evil.mint(alice, 1_000e18);
        vm.prank(alice);
        evil.approve(curveAddr, type(uint256).max);

        // Buy first, with the attack disabled, so alice actually holds tokens to sell.
        vm.prank(alice);
        evilCurve.buy(10e18, 0);
        uint256 held = evilToken.balanceOf(alice);
        assertGt(held, 0);

        // Now arm the attack on transfer(), which sell() triggers via safeTransfer.
        evil.setTarget(curveAddr);
        evil.setAttack(false, true);

        vm.prank(alice);
        vm.expectRevert();
        evilCurve.sell(held / 2, 0);
    }

    // -------------------------------------------------------------------- fuzz ---

    function testFuzz_BuyInvariant(uint256 stockIn) public {
        stockIn = bound(stockIn, 1e12, 10_000e18);

        uint256 tokensOut = curve.previewBuy(stockIn);
        assertLt(tokensOut, curve.VIRTUAL_TOKEN_RESERVE(), "can never buy the full virtual token reserve");

        vm.prank(alice);
        curve.buy(stockIn, 0);

        assertEq(token.balanceOf(alice), tokensOut);
        assertLe(curve.raised(), THRESHOLD * 3 + stockIn, "sanity bound");
    }

    function testFuzz_BuyThenSell_NeverProfitsAndNeverReverts(uint256 stockIn) public {
        // Keep well below the threshold so graduation doesn't fire mid-fuzz-run,
        // which would make the subsequent sell() revert by design.
        stockIn = bound(stockIn, 1e12, THRESHOLD / 4);

        vm.startPrank(alice);
        uint256 tokensOut = curve.previewBuy(stockIn);
        curve.buy(stockIn, 0);

        uint256 stockBack = curve.previewSell(tokensOut);
        curve.sell(tokensOut, 0);
        vm.stopPrank();

        assertLe(stockBack, stockIn, "fuzz: round trip must never profit");
        assertFalse(curve.graduated());
    }
}
