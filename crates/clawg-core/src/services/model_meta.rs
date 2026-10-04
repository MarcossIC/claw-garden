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
