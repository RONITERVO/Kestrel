//! Only the JSON tokenizer format pinned by Kestrel's seven-language manifest.
use anyhow::{Result, bail};
use std::sync::Arc;
pub struct Tokenizer(tokenizers::Tokenizer);
impl Tokenizer {
    pub fn load(n_bins: usize, path: &str, kind: &str) -> Result<Self> {
        if !matches!(kind, "tokenizers" | "json") {
            bail!("Unsupported speech tokenizer");
        }
        let tokenizer =
            tokenizers::Tokenizer::from_file(crate::download::download_if_necessary(path)?)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        if tokenizer.get_vocab_size(true) != n_bins {
            bail!("Speech tokenizer vocabulary mismatch");
        }
        Ok(Self(tokenizer))
    }
    pub fn encode(&self, text: &str) -> Result<Vec<u32>> {
        Ok(self
            .0
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .get_ids()
            .to_vec())
    }
    pub fn decode(&self, ids: &[u32]) -> Result<String> {
        self.0.decode(ids, true).map_err(|e| anyhow::anyhow!("{e}"))
    }
}
pub struct SharedTokenizer(pub Arc<Tokenizer>);
impl ptts::Tokenizer for SharedTokenizer {
    fn encode(&self, text: &str) -> xn::Result<Vec<u32>> {
        self.0
            .encode(text)
            .map_err(|e| xn::Error::msg(e.to_string()))
    }
    fn decode(&self, ids: &[u32]) -> xn::Result<String> {
        self.0
            .decode(ids)
            .map_err(|e| xn::Error::msg(e.to_string()))
    }
}
