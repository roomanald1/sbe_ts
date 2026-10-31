// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

// This runtime is injected ONLY into eth_call state overrides. It is never
// deployed, signed or broadcast. It measures the actual funded liquidation
// followed by the collateral->debt swap in the same simulated EVM transaction.
interface Token {
    function balanceOf(address) external view returns (uint256);
    function approve(address,uint256) external returns (bool);
}
interface Pool {
    function liquidationCall(address,address,address,uint256,bool) external;
}
interface SwapRouter {
    struct ExactInputSingleParams {
        address tokenIn; address tokenOut; uint24 fee; address recipient;
        uint256 deadline; uint256 amountIn; uint256 amountOutMinimum; uint160 sqrtPriceLimitX96;
    }
    function exactInputSingle(ExactInputSingleParams calldata) external payable returns(uint256);
}
contract PaperLiquidator {
    receive() external payable {}
    function simulateStaking(uint256 amount) external returns(uint256 receivedEth,uint256 gasUsed,uint256 minted) {
        uint256 startGas=gasleft();
        uint256 beforeEth=address(this).balance;
        address steth=0xae7ab96520DE3A18E5e111B5EaAb095312D7fE84;
        address curve=0xDC24316b9AE028F1497c275EB9192a3Ea0f67022;
        uint256 beforeToken=Token(steth).balanceOf(address(this));
        (bool deposited,)=steth.call{value:amount}(abi.encodeWithSignature("submit(address)",address(0)));
        require(deposited,"Lido submit");
        minted=Token(steth).balanceOf(address(this))-beforeToken;
        require(Token(steth).approve(curve,minted),"approve stETH");
        (bool swapped,)=curve.call(abi.encodeWithSignature(
            "exchange(int128,int128,uint256,uint256)",int128(1),int128(0),minted,uint256(0)));
        require(swapped,"Curve exchange");
        receivedEth=address(this).balance+amount-beforeEth;
        gasUsed=startGas-gasleft();
    }
    function probe() external pure returns(uint256) { return 1; }
    function simulate(address pool,address collateral,address debt,address user,
        uint256 repayment,address router,uint24 fee) external returns(uint256 receivedDebt,uint256 gasUsed,uint256 actualRepayment) {
        uint256 startGas=gasleft();
        uint256 beforeDebt=Token(debt).balanceOf(address(this));
        uint256 beforeCollateral=Token(collateral).balanceOf(address(this));
        require(Token(debt).approve(pool,type(uint256).max),"approve debt");
        Pool(pool).liquidationCall(collateral,debt,user,repayment,false);
        actualRepayment=beforeDebt-Token(debt).balanceOf(address(this));
        uint256 received=Token(collateral).balanceOf(address(this))-beforeCollateral;
        // USDC uses an optional return value on approve.
        (bool ok,bytes memory result)=collateral.call(
            abi.encodeWithSelector(Token.approve.selector,router,type(uint256).max));
        require(ok && (result.length==0 || abi.decode(result,(bool))),"approve collateral");
        receivedDebt=SwapRouter(router).exactInputSingle(SwapRouter.ExactInputSingleParams(
            collateral,debt,fee,address(this),block.timestamp,received,0,0));
        gasUsed=startGas-gasleft();
    }
}
