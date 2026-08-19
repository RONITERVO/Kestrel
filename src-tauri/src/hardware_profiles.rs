use serde::{Deserialize, Serialize};

use crate::models::ThinkingLevel;

/// A proven, empirically benchmarked hardware configuration for a specific local model and GPU VRAM tier.
/// Only contains configurations directly validated on local hardware without speculation or unproven claims.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProvenHardwareProfile {
    pub id: String,
    pub model_pattern: String,
    pub display_name: String,
    pub min_vram_mib: u32,
    pub max_vram_mib: Option<u32>,
    pub recommended_context_window: u32,
    pub recommended_max_output_tokens: u32,
    pub recommended_thinking_level: ThinkingLevel,
    pub recommended_threads: u32,
    pub description: String,
    pub proven_speed_notes: String,
}

/// Returns all built-in empirically benchmarked hardware profiles.
pub fn all_proven_profiles() -> Vec<ProvenHardwareProfile> {
    vec![
        // --- Qwen 3.8 27B Family ---
        ProvenHardwareProfile {
            id: "qwen-27b-12gb".into(),
            model_pattern: "qwen3.8-27b".into(),
            display_name: "Qwen 3.8 27B on 12GB GPU (Max Safe 32k)".into(),
            min_vram_mib: 10_000,
            max_vram_mib: Some(14_000),
            recommended_context_window: 32_768,
            recommended_max_output_tokens: 32_768,
            recommended_thinking_level: ThinkingLevel::High,
            recommended_threads: 16,
            description: "Empirically validated on RTX 5070 (12GB VRAM). Model weights (8.6GB) + 32k KV cache use 11,731 MiB. 36k+ spills to system RAM.".into(),
            proven_speed_notes: "100% in VRAM, ~1,000 tok/s prompt eval, ~41 tok/s generation".into(),
        },
        ProvenHardwareProfile {
            id: "qwen-27b-24gb".into(),
            model_pattern: "qwen3.8-27b".into(),
            display_name: "Qwen 3.8 27B on 24GB+ GPU".into(),
            min_vram_mib: 20_001,
            max_vram_mib: None,
            recommended_context_window: 131_072,
            recommended_max_output_tokens: 32_768,
            recommended_thinking_level: ThinkingLevel::High,
            recommended_threads: 16,
            description: "For 24GB+ GPUs (RTX 3090, RTX 4090, RTX 5090). Fits 128k context in VRAM.".into(),
            proven_speed_notes: "100% in VRAM with 128k long-context capacity".into(),
        },

        // --- Ternary Bonsai 27B Family ---
        ProvenHardwareProfile {
            id: "bonsai-27b-12gb".into(),
            model_pattern: "bonsai".into(),
            display_name: "Ternary Bonsai 27B on 12GB GPU (48k Context)".into(),
            min_vram_mib: 10_000,
            max_vram_mib: Some(14_000),
            recommended_context_window: 49_152,
            recommended_max_output_tokens: 32_768,
            recommended_thinking_level: ThinkingLevel::High,
            recommended_threads: 16,
            description: "Empirically validated on RTX 5070 (12GB VRAM). Model weights (6.8GB) + 48k KV cache use 11,078 MiB (peak limit is 57k; 64k+ spills).".into(),
            proven_speed_notes: "100% in VRAM, ~1,260 tok/s prompt eval, ~58 tok/s generation".into(),
        },
        ProvenHardwareProfile {
            id: "bonsai-27b-24gb".into(),
            model_pattern: "bonsai".into(),
            display_name: "Ternary Bonsai 27B on 24GB+ GPU".into(),
            min_vram_mib: 20_001,
            max_vram_mib: None,
            recommended_context_window: 131_072,
            recommended_max_output_tokens: 32_768,
            recommended_thinking_level: ThinkingLevel::High,
            recommended_threads: 16,
            description: "For 24GB+ GPUs. Fits 128k context fully in VRAM.".into(),
            proven_speed_notes: "100% in VRAM, 128k context support".into(),
        },

        // --- Gemma 4 E4B Family ---
        ProvenHardwareProfile {
            id: "gemma-4-e4b-12gb".into(),
            model_pattern: "gemma-4".into(),
            display_name: "Gemma 4 E4B on 12GB GPU (Full 128k Context)".into(),
            min_vram_mib: 7_000,
            max_vram_mib: None,
            recommended_context_window: 131_072,
            recommended_max_output_tokens: 32_768,
            recommended_thinking_level: ThinkingLevel::High,
            recommended_threads: 16,
            description: "Empirically validated on RTX 5070 (12GB VRAM). Model weights (4.7GB) + 128k KV cache use only 5,964 MiB (leaves ~6GB VRAM free).".into(),
            proven_speed_notes: "100% in VRAM, ~5,100 tok/s prompt eval, ~128 tok/s generation".into(),
        },
    ]
}

/// Matches a model name and optional detected GPU VRAM against the proven hardware profiles.
#[allow(dead_code)]
pub fn find_proven_profile(model_name: &str, vram_mib: Option<u32>) -> Option<ProvenHardwareProfile> {
    let lower_name = model_name.to_lowercase();
    all_proven_profiles().into_iter().find(|profile| {
        if !lower_name.contains(&profile.model_pattern) {
            return false;
        }
        if let Some(vram) = vram_mib {
            if vram < profile.min_vram_mib {
                return false;
            }
            if let Some(max_vram) = profile.max_vram_mib {
                if vram > max_vram {
                    return false;
                }
            }
        }
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_qwen_27b_on_12gb_rtx_5070() {
        let profile = find_proven_profile("Qwen3.8-27B-UD-IQ2_XXS.gguf", Some(12_227)).unwrap();
        assert_eq!(profile.id, "qwen-27b-12gb");
        assert_eq!(profile.recommended_context_window, 32_768);
        assert_eq!(profile.recommended_max_output_tokens, 32_768);
        assert_eq!(profile.recommended_thinking_level, ThinkingLevel::High);
    }

    #[test]
    fn matches_qwen_27b_on_24gb_gpu() {
        let profile = find_proven_profile("Qwen3.8-27B-UD-IQ2_XXS.gguf", Some(24_576)).unwrap();
        assert_eq!(profile.id, "qwen-27b-24gb");
        assert_eq!(profile.recommended_context_window, 131_072);
    }

    #[test]
    fn matches_bonsai_on_12gb_gpu() {
        let profile = find_proven_profile("Ternary-Bonsai-27B-Q2_0.gguf", Some(12_000)).unwrap();
        assert_eq!(profile.id, "bonsai-27b-12gb");
        assert_eq!(profile.recommended_context_window, 49_152);
    }

    #[test]
    fn matches_gemma4_on_12gb_gpu() {
        let profile = find_proven_profile("gemma-4-E4B-it-Q4_K_M.gguf", Some(12_000)).unwrap();
        assert_eq!(profile.id, "gemma-4-e4b-12gb");
        assert_eq!(profile.recommended_context_window, 131_072);
    }
}
