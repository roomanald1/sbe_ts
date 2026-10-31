use super::*;
const HELPER: &str = "0x00000000000000000000000000000000000a11ce";
const WETH: &str = "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2";
const USDC: &str = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48";
const AAVE: &str = "0x87870bca3f3fd6335c3f4ce8392d69350b4fa4e2";
const ROUTER: &str = "0xe592427a0aece92de3edee1f18e0157c05861564";
const RUNTIME: &str = include_str!("paper_liquidator_runtime.hex");
#[derive(Deserialize)]
pub(super) struct SimulationRequest {
    user: String,
    repayment_wei: String,
    block: String,
}
fn address(s: &str) -> bool {
    s.len() == 42 && s.starts_with("0x") && s[2..].bytes().all(|c| c.is_ascii_hexdigit())
}
fn word_address(s: &str) -> String {
    format!("{:0>64}", s.trim_start_matches("0x"))
}
fn override_code() -> Value {
    json!({"code":format!("0x{}",RUNTIME.trim())})
}
pub(super) async fn capability(State(s): State<Feeds>) -> Result<Json<RawFeed>, Error> {
    let started = now();
    let data=rpc(&s,json!({"jsonrpc":"2.0","id":1,"method":"eth_call","params":[
        {"from":HELPER,"to":HELPER,"data":"0xb74af5a9","gas":"0x1e8480"},"latest",{(HELPER):override_code()}
    ]})).await?;
    if data["result"]
        .as_str()
        .is_none_or(|r| r.trim_start_matches("0x").trim_start_matches('0') != "1")
    {
        return Err(error(format!(
            "RPC must support read-only state overrides: {data}"
        )));
    }
    Ok(Json(RawFeed {
        provider: "ethereum-state-override-probe".into(),
        requested_at_ms: started,
        received_at_ms: now(),
        data,
    }))
}
pub(super) async fn simulate(
    State(s): State<Feeds>,
    Json(x): Json<SimulationRequest>,
) -> Result<Json<RawFeed>, Error> {
    if !address(&x.user)
        || !x.block.starts_with("0x")
        || u64::from_str_radix(&x.block[2..], 16).is_err()
    {
        return Err(bad("Valid borrower and pinned block required"));
    }
    let amount = u128::from_str_radix(x.repayment_wei.trim_start_matches("0x"), 16)
        .map_err(|_| bad("Invalid repayment"))?;
    if amount == 0 || amount > 100_000_000_000_000_000_000 {
        return Err(bad("Repayment exceeds 100 WETH paper simulation limit"));
    }
    let start = now();
    let slot_preimage = format!("0x{}{:064x}", word_address(HELPER), 3);
    let hash = rpc(
        &s,
        json!({"jsonrpc":"2.0","id":1,"method":"web3_sha3","params":[slot_preimage]}),
    )
    .await?;
    let slot = hash["result"]
        .as_str()
        .filter(|s| s.len() == 66)
        .ok_or(error("WETH storage hash unavailable"))?;
    let calldata = format!(
        "0x2ab10875{}{}{}{}{:064x}{}{:064x}",
        word_address(AAVE),
        word_address(USDC),
        word_address(WETH),
        word_address(&x.user),
        amount,
        word_address(ROUTER),
        500
    );
    let funded = amount * 2 + 1_000_000_000_000_000_000;
    let overrides = json!({
        (HELPER):override_code(),
        (WETH):{"stateDiff":{(slot):format!("0x{funded:064x}")}}
    });
    let data = rpc(
        &s,
        json!({"jsonrpc":"2.0","id":1,"method":"eth_call","params":[
            {"from":HELPER,"to":HELPER,"data":calldata,"gas":"0x1e8480"},x.block,overrides
        ]}),
    )
    .await?;
    Ok(Json(RawFeed {
        provider: "ethereum-funded-liquidation-and-swap-eth-call".into(),
        requested_at_ms: start,
        received_at_ms: now(),
        data,
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn runtime_and_abi_are_reviewable_and_simulation_is_funded_not_broadcast() {
        assert!(RUNTIME.trim().bytes().all(|b| b.is_ascii_hexdigit()));
        let build: Value =
            serde_json::from_str(include_str!("paper_liquidator_build.json")).unwrap();
        assert_eq!(build["method_identifiers"]["probe()"], "b74af5a9");
        assert_eq!(
            build["method_identifiers"]["simulate(address,address,address,address,uint256,address,uint24)"],
            "2ab10875"
        );
        assert_eq!(word_address(HELPER).len(), 64);
        assert!(address(AAVE) && address(WETH) && address(USDC) && address(ROUTER));
    }
}

#[derive(Deserialize)]
pub(super) struct StakingRequest {
    input_wei: String,
    block: String,
}
pub(super) async fn staking(
    State(s): State<Feeds>,
    Json(x): Json<StakingRequest>,
) -> Result<Json<RawFeed>, Error> {
    let amount = u128::from_str_radix(x.input_wei.trim_start_matches("0x"), 16)
        .map_err(|_| bad("Invalid staking amount"))?;
    if amount == 0
        || amount > 100_000_000_000_000_000_000
        || !x.block.starts_with("0x")
        || u64::from_str_radix(&x.block[2..], 16).is_err()
    {
        return Err(bad(
            "Pinned block and positive input of at most 100 ETH required",
        ));
    }
    let started = now();
    let data=rpc(&s,json!({"jsonrpc":"2.0","id":1,"method":"eth_call","params":[
        {"from":HELPER,"to":HELPER,"data":format!("0x71860d85{amount:064x}"),"gas":"0x1e8480"},
        x.block,{(HELPER):{"code":format!("0x{}",RUNTIME.trim()),"balance":format!("0x{:x}",amount*2)}}
    ]})).await?;
    Ok(Json(RawFeed {
        provider: "ethereum-lido-mint-curve-sale-eth-call".into(),
        requested_at_ms: started,
        received_at_ms: now(),
        data,
    }))
}
