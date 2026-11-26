//! Subscription names as the menu shows them, from the providers' plan codes.

/// "max" with a `default_claude_max_20x` tier is "Max 20x".
pub(crate) fn claude(subscription: &str, tier: Option<&str>) -> String {
    let multiplier = tier.and_then(|tier| {
        ["20x", "5x"]
            .into_iter()
            .find(|multiplier| tier.ends_with(&format!("_{multiplier}")))
    });
    match (subscription, multiplier) {
        ("max", Some(multiplier)) => format!("Max {multiplier}"),
        _ => title_case(subscription),
    }
}

/// ChatGPT plan codes such as "plus" or "team".
pub(crate) fn codex(plan: &str) -> String {
    title_case(plan)
}

fn title_case(code: &str) -> String {
    code.split(['_', '-', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_max_shows_its_multiplier() {
        assert_eq!(claude("max", Some("default_claude_max_5x")), "Max 5x");
        assert_eq!(claude("max", Some("default_claude_max_20x")), "Max 20x");
        assert_eq!(claude("max", None), "Max");
        assert_eq!(claude("pro", Some("default_claude_ai")), "Pro");
        // A Team seat can report a Max tier; the subscription decides.
        assert_eq!(claude("team", Some("default_claude_max_5x")), "Team");
    }

    #[test]
    fn codex_plans_are_title_cased() {
        assert_eq!(codex("plus"), "Plus");
        assert_eq!(codex("free"), "Free");
        assert_eq!(codex("self_serve_business"), "Self Serve Business");
    }
}
