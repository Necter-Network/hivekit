"""
NRC-1 example — price oracle consensus module (Python)

Build:  hivec build price_oracle.py
Run:    hivec run price_oracle.py __consensus '{"pair": "ETH/USD", "__round": 1}'

The reward configuration is used by the SDK's consensus wrapper; it is not part
of the v1 manifest.
"""

from hivekit import hive, NRC1Config, ConsensusContext, ConsensusResult


@hive.consensus(NRC1Config(
    reward_token="0xYourOracleTokenAddress",
    reward_chain="base",
    reward_per_unit="100000000000000000",  # 0.1 ORACLE per CU
    token_standard="ERC-20",
    units_per_execution=10,
    display_name="Price Oracle",
    description="Decentralized ETH/USD price aggregation",
))
def oracle_handler(ctx: ConsensusContext) -> ConsensusResult:
    pair = ctx.input.get("pair", "ETH/USD")

    # In production: fetch price from a trusted source in ctx.input
    price = 3200

    return ConsensusResult(
        output={"pair": pair, "price": price, "round": ctx.round},
        compute_units=10,
    )


if __name__ == "__main__":
    # Local test
    result = hive.invoke_local("__consensus", {
        "__round": 1,
        "__participants": ["0xMiner1", "0xMiner2"],
        "pair": "ETH/USD",
    })
    print("Consensus result:", result)
