//! Bounded local token observations. No arbitrary CJK n-grams or correction rules.
use std::collections::HashSet;

fn cjk(c: char) -> bool {
    ('\u{3400}'..='\u{9fff}').contains(&c)
}
fn code_chunk(chunk: &str) -> bool {
    let chunk = chunk.trim_matches(|c: char| ",，。.!?！？\"'“”‘’".contains(c));
    chunk.contains(['/', '\\', ':', '=', '`', '(', ')', '{', '}', '[', ']', ';'])
        || chunk.contains('@')
        || chunk.contains('.')
}

pub fn extract(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |term: &str| {
        let term =
            term.trim_matches(|c: char| c.is_whitespace() || ",，。!?！？\"'“”‘’".contains(c));
        if (2..=64).contains(&term.chars().count())
            && seen.insert(term.to_lowercase())
            && out.len() < 256
        {
            out.push(term.to_owned());
        }
    };
    for line in text.lines().take(1000) {
        if line.contains(['`', '{', '}']) {
            continue;
        }
        for chunk in line.split_whitespace() {
            if code_chunk(chunk) {
                continue;
            }
            // Reuse the existing technical-token policy, preserving C++ vs C#.
            for token in chunk.split(|c: char| !(c.is_ascii_alphanumeric() || "_+#-".contains(c))) {
                if token.is_empty() {
                    continue;
                }
                let proposals = super::core::extract_terms(token);
                if !proposals.is_empty()
                    || matches!(
                        token.to_ascii_lowercase().as_str(),
                        "rust"
                            | "sqlite"
                            | "python"
                            | "typescript"
                            | "javascript"
                            | "docker"
                            | "kubernetes"
                            | "postgresql"
                            | "codex"
                            | "claude"
                            | "code"
                            | "api"
                    )
                {
                    add(token);
                }
            }
            {
                static JIEBA: std::sync::OnceLock<jieba_rs::Jieba> = std::sync::OnceLock::new();
                let jieba = JIEBA.get_or_init(jieba_rs::Jieba::new);
                for token in jieba.cut(chunk, true) {
                    let mut term = token.word;
                    // HMM may attach the locative particle in “在…里” to an
                    // unknown name. Strip only in that grammatical context,
                    // never generate arbitrary substrings or a product whitelist.
                    if chunk[..token.byte_start].ends_with('在') && term.chars().count() >= 3 {
                        term = term.strip_suffix('里').unwrap_or(term);
                    }
                    if term.chars().all(cjk)
                        && (2..=12).contains(&term.chars().count())
                        && !matches!(
                            term,
                            "今天"
                                | "明天"
                                | "我们"
                                | "他们"
                                | "这个"
                                | "那个"
                                | "可以"
                                | "需要"
                                | "不要"
                                | "使用"
                                | "进行"
                                | "一个"
                                | "什么"
                                | "怎么"
                                | "已经"
                                | "还是"
                                | "就是"
                        )
                    {
                        add(term);
                    }
                }
            }
        }
    }
    out
}
