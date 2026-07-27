// SPDX-License-Identifier: MIT
pragma solidity ^0.8.24;

import {Test} from "forge-std/Test.sol";
import {SimplePool} from "../src/SimplePool.sol";
import {MockERC20} from "./mocks/MockERC20.sol";

contract SimplePoolTest is Test {
    SimplePool internal pool;
    MockERC20 internal tokenA;
    MockERC20 internal tokenB;

    address internal alice = makeAddr("alice");
    address internal bob = makeAddr("bob");

    function setUp() public {
        tokenA = new MockERC20("Token A", "TKA");
        tokenB = new MockERC20("Token B", "TKB");
        pool = new SimplePool(address(tokenA), address(tokenB));
    }

    function _seedInitialLiquidity(uint256 amountA, uint256 amountB, address to) internal returns (uint256 liquidity) {
        tokenA.mint(address(pool), amountA);
        tokenB.mint(address(pool), amountB);
        liquidity = pool.mint(to);
    }

    function test_Mint_FirstDeposit_LocksMinimumLiquidity() public {
        uint256 liquidity = _seedInitialLiquidity(1000e18, 1000e18, alice);

        assertEq(pool.balanceOf(address(0)), pool.MINIMUM_LIQUIDITY());
        assertEq(pool.balanceOf(alice), liquidity);
        assertEq(pool.totalSupply(), liquidity + pool.MINIMUM_LIQUIDITY());
        (uint256 r0, uint256 r1) = pool.getReserves();
        assertEq(r0, 1000e18);
        assertEq(r1, 1000e18);
    }

    function test_Mint_Proportional_SecondDeposit() public {
        _seedInitialLiquidity(1000e18, 1000e18, alice);

        tokenA.mint(address(pool), 100e18);
        tokenB.mint(address(pool), 100e18);
        uint256 liquidity2 = pool.mint(bob);

        // Depositing 10% more of both sides should mint ~10% more LP.
        uint256 totalBefore = pool.totalSupply() - liquidity2;
        assertApproxEqRel(liquidity2, totalBefore / 10, 0.01e18);
    }

    function test_Swap_ConstantProductInvariantHolds() public {
        _seedInitialLiquidity(10_000e18, 10_000e18, alice);

        uint256 amountIn = 100e18;
        tokenA.mint(address(this), amountIn);
        tokenA.transfer(address(pool), amountIn);

        (uint256 r0, uint256 r1) = pool.getReserves();
        // amountOut via the 0.3%-fee constant product formula.
        uint256 amountInWithFee = amountIn * 997;
        uint256 amountOut = (amountInWithFee * r1) / (r0 * 1000 + amountInWithFee);

        uint256 before = tokenB.balanceOf(bob);
        pool.swap(0, amountOut, bob);
        assertEq(tokenB.balanceOf(bob) - before, amountOut);

        (uint256 newR0, uint256 newR1) = pool.getReserves();
        assertGe(newR0 * newR1, r0 * r1, "K must not decrease after a fee-paying swap");
    }

    function test_Swap_RevertsIfInvariantViolated() public {
        _seedInitialLiquidity(10_000e18, 10_000e18, alice);

        // Try to take out far more than the input justifies.
        uint256 amountIn = 1e18;
        tokenA.mint(address(this), amountIn);
        tokenA.transfer(address(pool), amountIn);

        vm.expectRevert(bytes("K invariant"));
        pool.swap(0, 50e18, bob);
    }

    function test_Burn_ReturnsProportionalShare() public {
        uint256 liquidity = _seedInitialLiquidity(1000e18, 1000e18, alice);

        vm.prank(alice);
        pool.transfer(address(pool), liquidity);

        uint256 aliceABefore = tokenA.balanceOf(alice);
        uint256 aliceBBefore = tokenB.balanceOf(alice);

        (uint256 amount0, uint256 amount1) = pool.burn(alice);

        assertEq(tokenA.balanceOf(alice) - aliceABefore, amount0);
        assertEq(tokenB.balanceOf(alice) - aliceBBefore, amount1);
        assertGt(amount0, 0);
        assertGt(amount1, 0);
    }
}
