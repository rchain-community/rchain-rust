fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        // `New.injections` is a `map<string, Par>` that participates in the content-addressed
        // state hash. Generate it as a `BTreeMap` so protobuf encoding iterates keys in sorted
        // order (a `HashMap` would make the post-state hash depend on process hash-seed order).
        .btree_map(".rholang.New")
        .compile_protos(
            &[
                "proto/casper.proto",
                "proto/routing.proto",
                "proto/kademlia.proto",
                "proto/RhoTypes.proto",
                "proto/service_error.proto",
                "proto/propose_service_common.proto",
                "proto/propose_service_v1.proto",
                "proto/deploy_service_common.proto",
                "proto/deploy_service_v1.proto",
                "proto/repl.proto",
            ],
            &["proto"],
        )?;
    Ok(())
}
