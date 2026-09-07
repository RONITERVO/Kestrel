use serde::Serialize;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RuntimePolicyTier {
    pub maximum_context_window: u32,
    pub maximum_max_output_tokens: u32,
}

#[derive(Debug, Clone, Copy, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RuntimePolicyCatalog {
    pub minimum_context_window: u32,
    pub minimum_max_output_tokens: u32,
    pub standard: RuntimePolicyTier,
    pub advanced: RuntimePolicyTier,
}

pub const RUNTIME_POLICY_CATALOG: RuntimePolicyCatalog = RuntimePolicyCatalog {
    minimum_context_window: 4096,
    minimum_max_output_tokens: 1024,
    standard: RuntimePolicyTier {
        maximum_context_window: 98304,
        maximum_max_output_tokens: 32768,
    },
    advanced: RuntimePolicyTier {
        maximum_context_window: 1048576,
        maximum_max_output_tokens: 262144,
    },
};

#[derive(Clone, Copy)]
pub struct RuntimePolicyLimits {
    pub minimum_context_window: u32,
    pub minimum_max_output_tokens: u32,
    pub maximum_context_window: u32,
    pub maximum_max_output_tokens: u32,
}

pub fn runtime_policy_limits(advanced: bool) -> RuntimePolicyLimits {
    let limits = RUNTIME_POLICY_CATALOG;
    let tier = if advanced {
        limits.advanced
    } else {
        limits.standard
    };
    RuntimePolicyLimits {
        minimum_context_window: limits.minimum_context_window,
        minimum_max_output_tokens: limits.minimum_max_output_tokens,
        maximum_context_window: tier.maximum_context_window,
        maximum_max_output_tokens: tier.maximum_max_output_tokens,
    }
}
