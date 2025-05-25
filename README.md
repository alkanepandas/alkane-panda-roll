# Alkane Panda Roll

Roll the Pandas.

## Building

```bash
cargo build --target wasm32-unknown-unknown --release
```

The compiled WASM binary will be available in `target/wasm32-unknown-unknown/release/alkane_pandas_roll.wasm`. 

## Deployment

```bash
oyl alkane new-contract -c ./target/alkanes/wasm32-unknown-unknown/release/alkane_pandas_roll.wasm -data 1,0 -p oylnet
```

## Tracing

```bash
oyl provider alkanes --method trace -params '{"txid":"db7d367255ae3ddff3e4b714e9113c1402b91975df5d50d0c23aa36caff20697", "vout":3}' -p oylnet
``` 