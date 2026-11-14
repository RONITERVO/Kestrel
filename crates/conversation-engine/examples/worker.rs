//! Standalone entry for explicit native speech acceptance tests; the app uses the same function.
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let first = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("Missing asset root"))?;
    let root = if first == "--conversation-voice-worker" {
        args.next()
            .ok_or_else(|| anyhow::anyhow!("Missing asset root"))?
    } else {
        first
    };
    kestrel_conversation_engine::worker::run(root.into())
}
