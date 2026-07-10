/// Returns max output tokens for a given model.
pub fn max_output_tokens(provider: &str, model: &str) -> u32 {
    match provider {
        "anthropic" => anthropic_max_tokens(model),
        "openrouter" => openrouter_max_tokens(model),
        "ollama" => 4096,
        _ => 4096,
    }
}

fn anthropic_max_tokens(model: &str) -> u32 {
    if model.contains("opus") {
        32_000
    } else if model.contains("sonnet") || model.contains("haiku") {
        16_000
    } else {
        8_192
    }
}

fn openrouter_max_tokens(model: &str) -> u32 {
    if model.contains("claude") {
        16_000
    } else if model.contains("gpt-4") {
        16_384
    } else if model.contains("gpt-3.5") {
        4_096
    } else if model.contains("llama") || model.contains("mistral") {
        4_096
    } else {
        4_096
    }
}

/// Resolves model alias to canonical model ID.
/// If not a known alias, returns the input as-is.
pub fn resolve_alias(alias: &str) -> &str {
    match alias {
        // Anthropic
        "opus" => "claude-opus-4-6",
        "sonnet" => "claude-sonnet-4-6",
        "haiku" => "claude-haiku-4-5-20251001",

        // Ollama shortcuts
        "llama" | "llama3" => "llama3.1",
        "mistral" => "mistral:latest",
        "codellama" => "codellama:latest",
        "deepseek" => "deepseek-coder:latest",

        // Pass through
        _ => alias,
    }
}

/// Estimated context window (for auto-compaction threshold).
pub fn estimated_context_window(provider: &str, model: &str) -> usize {
    match provider {
        "anthropic" => 200_000,
        "ollama" => 32_000,
        "openrouter" => {
            if model.contains("claude") {
                200_000
            } else if model.contains("gpt-4o") || model.contains("gpt-4") {
                128_000
            } else {
                32_000
            }
        }
        _ => 32_000,
    }
}
