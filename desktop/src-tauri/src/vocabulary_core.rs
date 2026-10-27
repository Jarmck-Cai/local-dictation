//! Bounded, local-only vocabulary proposals and conservative confirmed replacements.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    pub term: String,
    pub replacement_from: Option<String>,
    pub context: Option<String>,
    pub source: String,
    pub occurrences: u32,
}

#[derive(Clone, Debug)]
pub struct Rule {
    pub id: i64,
    pub term: String,
    pub aliases: Vec<String>,
    pub context: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CorrectionHit {
    pub rule_index: usize,
    pub term_id: i64,
    pub term: String,
    pub from: String,
    pub count: u32,
}

pub struct AppliedRules {
    pub text: String,
    pub hits: Vec<CorrectionHit>,
}

/// Only bounded term/alias fragments are retained, never the transcript body.
#[derive(Default)]
pub struct RecentCorrections {
    pub hits: Vec<CorrectionHit>,
    pub recorded_at_ms: Option<u64>,
}

impl RecentCorrections {
    pub fn record(&mut self, mut hits: Vec<CorrectionHit>) {
        hits.truncate(64);
        self.hits = hits;
        self.recorded_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64);
    }
}

/// Validate explicitly edited rules with the same conservative alias policy.
pub fn validate_rule(rule: &Rule) -> Result<(), &'static str> {
    let allowed = |c: char| c.is_alphanumeric() || c == ' ' || "_./+-#".contains(c);
    if rule.term.is_empty() || rule.term.chars().count() > 64 || !rule.term.chars().all(allowed) {
        return Err("vocabulary_term_invalid");
    }
    if rule.context.as_ref().is_some_and(|context| {
        !(2..=24).contains(&context.chars().count())
            || !context
                .chars()
                .all(|c| c.is_alphanumeric() || c == ' ' || "_+-".contains(c))
    }) {
        return Err("vocabulary_context_invalid");
    }
    if rule.aliases.len() > 20 {
        return Err("vocabulary_alias_limit");
    }
    for (index, alias) in rule.aliases.iter().enumerate() {
        if !alias.chars().all(allowed)
            || !safe_alias(alias, &rule.term, rule.context.as_deref())
            || alias.eq_ignore_ascii_case(&rule.term)
        {
            return Err("vocabulary_alias_invalid");
        }
        if rule.aliases[..index]
            .iter()
            .any(|other| other.eq_ignore_ascii_case(alias))
        {
            return Err("vocabulary_alias_duplicate");
        }
    }
    Ok(())
}

pub const MAX_SAMPLE_CHARS: usize = 8_000;
const MAX_DIFF_CELLS: usize = 1_000_000;
const MAX_PROPOSALS: usize = 40;
const DOMAIN_SUFFIXES: &[&str] = &[
    "模型",
    "接口",
    "框架",
    "协议",
    "数据库",
    "编译器",
    "处理器",
    "词库",
    "算法",
    "终端",
];

fn is_cjk(c: char) -> bool {
    ('\u{3400}'..='\u{9fff}').contains(&c)
}

fn common_word(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "a" | "an"
            | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "but"
            | "by"
            | "can"
            | "could"
            | "do"
            | "for"
            | "from"
            | "have"
            | "i"
            | "if"
            | "in"
            | "is"
            | "it"
            | "let"
            | "like"
            | "may"
            | "me"
            | "my"
            | "of"
            | "on"
            | "open"
            | "or"
            | "our"
            | "please"
            | "run"
            | "see"
            | "should"
            | "so"
            | "that"
            | "the"
            | "their"
            | "them"
            | "then"
            | "there"
            | "these"
            | "this"
            | "to"
            | "use"
            | "very"
            | "was"
            | "we"
            | "will"
            | "with"
            | "would"
            | "you"
            | "your"
            | "可能"
            | "确定"
            | "整理"
            | "总结"
            | "需要"
            | "可以"
            | "执行"
            | "明天"
            | "今天"
            | "我们"
            | "他们"
            | "这个"
            | "那个"
    )
}

fn protected(value: &str) -> bool {
    value
        .chars()
        .any(|c| c.is_numeric() || "零〇一二三四五六七八九十百千万亿不没无别勿非".contains(c))
        || value.split_whitespace().any(|word| {
            matches!(
                word.to_ascii_lowercase().as_str(),
                "not"
                    | "no"
                    | "never"
                    | "cannot"
                    | "don't"
                    | "can't"
                    | "maybe"
                    | "certainly"
                    | "possibly"
            )
        })
}

fn is_latin_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "_./+-#".contains(c)
}

fn tokens(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut latin_start = None;
    for (i, c) in text.char_indices() {
        if is_latin_word_char(c) {
            latin_start.get_or_insert(i);
        } else {
            if let Some(start) = latin_start.take() {
                out.push((start, i));
            }
            out.push((i, i + c.len_utf8()));
        }
    }
    if let Some(start) = latin_start {
        out.push((start, text.len()));
    }
    out
}

fn technical_word(word: &str) -> bool {
    word.len() >= 2
        && word.len() <= 64
        && word.is_ascii()
        && word.chars().any(|c| c.is_ascii_alphabetic())
        && !common_word(word)
        && (word.chars().any(|c| c.is_ascii_uppercase())
            || word.contains(['_', '.', '/', '+', '#']))
}

fn add_proposal(out: &mut Vec<Proposal>, proposal: Proposal) {
    if let Some(existing) = out.iter_mut().find(|p| {
        p.term == proposal.term
            && p.replacement_from == proposal.replacement_from
            && p.context == proposal.context
    }) {
        existing.occurrences = existing.occurrences.saturating_add(proposal.occurrences);
    } else if out.len() < MAX_PROPOSALS {
        out.push(proposal);
    }
}

pub fn extract_terms(text: &str) -> Vec<Proposal> {
    if text.chars().count() > MAX_SAMPLE_CHARS {
        return Vec::new();
    }
    let parts = tokens(text);
    let mut out = Vec::new();
    for (index, &(start, end)) in parts.iter().enumerate() {
        let word = &text[start..end];
        if !technical_word(word) {
            continue;
        }
        add_proposal(
            &mut out,
            Proposal {
                term: word.into(),
                replacement_from: None,
                context: None,
                source: "transcript".into(),
                occurrences: 1,
            },
        );
        if let (Some(&(gap_start, gap_end)), Some(&(next_start, next_end))) =
            (parts.get(index + 1), parts.get(index + 2))
        {
            let next = &text[next_start..next_end];
            if text[gap_start..gap_end].chars().all(char::is_whitespace) && technical_word(next) {
                add_proposal(
                    &mut out,
                    Proposal {
                        term: format!("{word} {next}"),
                        replacement_from: None,
                        context: None,
                        source: "transcript".into(),
                        occurrences: 1,
                    },
                );
            }
        }
    }
    out
}

fn context_near(text: &str, start: usize, end: usize) -> Option<String> {
    let nearby = |value: &str, reverse: bool| -> Option<String> {
        let mut parts = tokens(value);
        if reverse {
            parts.reverse();
        }
        let mut characters = 0;
        let (mut first, mut last) = if reverse {
            (value.len(), value.len())
        } else {
            (0, 0)
        };
        for (token_start, token_end) in parts {
            let token = &value[token_start..token_end];
            if !token.chars().all(|c| {
                c.is_alphanumeric()
                    || c.is_whitespace() && c != '\n' && c != '\r'
                    || "_+-".contains(c)
            }) {
                break;
            }
            let count = token.chars().count();
            // Context matching has a 24-character window. Never create a
            // fragment of an ASCII token that its word-boundary check rejects.
            if characters + count > 24 {
                break;
            }
            characters += count;
            first = first.min(token_start);
            last = last.max(token_end);
            if characters >= 6 {
                break;
            }
        }
        let context = value[first..last].trim();
        (context.chars().count() >= 2).then(|| context.to_owned())
    };
    nearby(&text[..start], true).or_else(|| nearby(&text[end..], false))
}

fn safe_alias(from: &str, term: &str, context: Option<&str>) -> bool {
    let from = from.trim();
    let term = term.trim();
    if from.is_empty()
        || term.is_empty()
        || from == term
        || protected(from)
        || protected(term)
        || common_word(from)
        || common_word(term)
        || from.chars().count() > 24
        || term.chars().count() > 40
        || !from
            .chars()
            .all(|c| c.is_alphanumeric() || c.is_whitespace() || "_./+-#".contains(c))
        || !term
            .chars()
            .all(|c| c.is_alphanumeric() || c.is_whitespace() || "_./+-#".contains(c))
    {
        return false;
    }
    if !from.is_ascii() && context.is_none_or(|s| s.chars().count() < 2) {
        return false;
    }
    if term.chars().any(|c| c.is_ascii_alphabetic())
        && !term.split_whitespace().any(|word| {
            technical_word(word)
                || matches!(
                    word,
                    "python"
                        | "typescript"
                        | "javascript"
                        | "rust"
                        | "kubernetes"
                        | "pytorch"
                        | "docker"
                        | "sqlite"
                        | "postgresql"
                        | "fastapi"
                )
        })
    {
        return false;
    }
    if !from.chars().any(|c| c.is_ascii_alphabetic())
        && !term.chars().any(|c| c.is_ascii_alphabetic())
    {
        // Chinese-only phrase changes are too ambiguous unless the target has a
        // domain noun suffix. Unknown names can still be confirmed as terms.
        return term.chars().count() >= 2
            && term.chars().count() <= 12
            && DOMAIN_SUFFIXES.iter().any(|suffix| term.ends_with(suffix));
    }
    true
}

fn expand_chinese_domain_change(
    before: &str,
    corrected: &str,
    start: usize,
    end: usize,
    fixed_start: usize,
    fixed_end: usize,
) -> (usize, usize, usize, usize) {
    if !before[start..end].chars().all(is_cjk)
        || !corrected[fixed_start..fixed_end].chars().all(is_cjk)
    {
        return (start, end, fixed_start, fixed_end);
    }
    let left: String = corrected[..fixed_start]
        .chars()
        .rev()
        .take_while(|c| is_cjk(*c) && !"的在用把将让与和是对要".contains(*c))
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if !before[..start].ends_with(&left) {
        return (start, end, fixed_start, fixed_end);
    }
    let expanded_start = fixed_start - left.len();
    let mut right_len = 0;
    for c in corrected[fixed_end..].chars().take(8) {
        if !is_cjk(c) || "的在用把将让与和是对要".contains(c) {
            break;
        }
        right_len += c.len_utf8();
        let target = &corrected[expanded_start..fixed_end + right_len];
        if DOMAIN_SUFFIXES
            .iter()
            .any(|suffix| target.ends_with(suffix))
            && before[end..].starts_with(&corrected[fixed_end..fixed_end + right_len])
        {
            return (
                start - left.len(),
                end + right_len,
                expanded_start,
                fixed_end + right_len,
            );
        }
    }
    (start, end, fixed_start, fixed_end)
}

fn compare_one(raw: &str, before: &str, corrected: &str, out: &mut Vec<Proposal>) {
    if before == corrected {
        return;
    }
    let a = tokens(before);
    let b = tokens(corrected);
    if (a.len() + 1).saturating_mul(b.len() + 1) > MAX_DIFF_CELLS {
        return;
    }
    let width = b.len() + 1;
    let mut lcs = vec![0u16; (a.len() + 1) * width];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i * width + j] = if before[a[i].0..a[i].1] == corrected[b[j].0..b[j].1] {
                1 + lcs[(i + 1) * width + j + 1]
            } else {
                lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
            };
        }
    }
    let unchanged = usize::from(lcs[0]);
    // A changed sentence is a useful sample, but is not an automatic alias.
    if unchanged < 3 || (a.len() + b.len() - unchanged * 2) > ((a.len() + b.len()) / 3).max(6) {
        return;
    }
    let (mut i, mut j) = (0, 0);
    let mut changes = Vec::new();
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && before[a[i].0..a[i].1] == corrected[b[j].0..b[j].1] {
            i += 1;
            j += 1;
            continue;
        }
        let (a_start, b_start) = (i, j);
        while i < a.len() || j < b.len() {
            if i < a.len() && j < b.len() && before[a[i].0..a[i].1] == corrected[b[j].0..b[j].1] {
                break;
            }
            if j < b.len() && (i == a.len() || lcs[i * width + j + 1] >= lcs[(i + 1) * width + j]) {
                j += 1;
            } else {
                i += 1;
            }
        }
        if i > a_start && j > b_start {
            changes.push((a[a_start].0, a[i - 1].1, b[b_start].0, b[j - 1].1));
        }
    }
    if changes.len() > 8 {
        return;
    }
    for (start, end, fixed_start, fixed_end) in changes {
        let (start, end, fixed_start, fixed_end) =
            expand_chinese_domain_change(before, corrected, start, end, fixed_start, fixed_end);
        let from = before[start..end].trim();
        let term = corrected[fixed_start..fixed_end].trim();
        let context = context_near(before, start, end);
        if safe_alias(from, term, context.as_deref()) {
            add_proposal(
                out,
                Proposal {
                    term: term.into(),
                    replacement_from: Some(from.into()),
                    context,
                    source: if raw.contains(from) {
                        "asr_correction"
                    } else {
                        "cleanup_correction"
                    }
                    .into(),
                    occurrences: 1,
                },
            );
        }
    }
}

pub fn correction_candidates(raw: &str, actual: &str, corrected: &str) -> Vec<Proposal> {
    if [raw, actual, corrected]
        .iter()
        .any(|s| s.chars().count() > MAX_SAMPLE_CHARS)
    {
        return Vec::new();
    }
    let mut out = Vec::new();
    compare_one(raw, actual, corrected, &mut out);
    if raw != actual {
        compare_one(raw, raw, corrected, &mut out);
    }
    for mut proposal in extract_terms(corrected) {
        proposal.source = "correction".into();
        add_proposal(&mut out, proposal);
    }
    out
}

fn matches_at(text: &str, needle: &str, index: usize) -> bool {
    if needle.is_empty()
        || !text.is_char_boundary(index)
        || !text.is_char_boundary(index + needle.len())
    {
        return false;
    }
    let Some(slice) = text.get(index..index + needle.len()) else {
        return false;
    };
    if !slice.eq_ignore_ascii_case(needle) {
        return false;
    }
    let ascii_edge = |c: char| c.is_ascii_alphanumeric() || "_./+-#".contains(c);
    !(needle.starts_with(ascii_edge) && text[..index].chars().next_back().is_some_and(ascii_edge)
        || needle.ends_with(ascii_edge)
            && text[index + needle.len()..]
                .chars()
                .next()
                .is_some_and(ascii_edge))
}

fn context_matches_at(text: &str, context: Option<&str>, start: usize, end: usize) -> bool {
    let Some(context) = context else {
        return true;
    };
    if context.is_empty() {
        return false;
    }
    // Context belongs to this occurrence, never to another sentence or clause.
    // The character bound also protects unpunctuated speech and mixed UTF-8 text.
    const CONTEXT_WINDOW_CHARS: usize = 24;
    let boundary = |c: char| "。！？!?\n\r，,；;：:.".contains(c);
    let left_start = text[..start]
        .char_indices()
        .rev()
        .take_while(|(_, c)| !boundary(*c))
        .take(CONTEXT_WINDOW_CHARS)
        .last()
        .map_or(start, |(index, _)| index);
    let right_end = text[end..]
        .char_indices()
        .take_while(|(_, c)| !boundary(*c))
        .take(CONTEXT_WINDOW_CHARS)
        .last()
        .map_or(end, |(index, c)| end + index + c.len_utf8());
    [&text[left_start..start], &text[end..right_end]]
        .iter()
        .any(|nearby| {
            nearby
                .char_indices()
                .any(|(index, _)| matches_at(nearby, context, index))
        })
}

fn contextual_match_at(text: &str, alias: &str, context: Option<&str>, index: usize) -> bool {
    matches_at(text, alias, index) && context_matches_at(text, context, index, index + alias.len())
}

#[cfg(test)]
pub fn apply_rules(text: &str, rules: &[Rule]) -> String {
    apply_rules_with_hits(text, rules).text
}

pub fn apply_rules_with_hits(text: &str, rules: &[Rule]) -> AppliedRules {
    let mut replacements: Vec<(usize, usize, &str, usize)> = Vec::new();
    for (rule_index, rule) in rules.iter().enumerate() {
        for alias in rule
            .aliases
            .iter()
            .filter(|alias| safe_alias(alias, &rule.term, rule.context.as_deref()))
            .map(String::as_str)
            .chain(std::iter::once(rule.term.as_str()))
        {
            for (index, _) in text.char_indices() {
                if contextual_match_at(text, alias, rule.context.as_deref(), index) {
                    replacements.push((index, index + alias.len(), &rule.term, rule_index));
                }
            }
        }
    }
    replacements.sort_by(|a, b| a.0.cmp(&b.0).then((b.1 - b.0).cmp(&(a.1 - a.0))));
    let mut result = String::with_capacity(text.len());
    let mut hits: Vec<CorrectionHit> = Vec::new();
    let mut consumed = 0;
    for &(start, end, term, rule_index) in &replacements {
        if start < consumed
            || replacements
                .iter()
                .any(|other| other.0 == start && other.1 == end && other.2 != term)
        {
            continue;
        }
        result.push_str(&text[consumed..start]);
        result.push_str(term);
        let from = &text[start..end];
        if from != term {
            if let Some(hit) = hits
                .iter_mut()
                .find(|hit| hit.rule_index == rule_index && hit.from == from)
            {
                hit.count = hit.count.saturating_add(1);
            } else {
                hits.push(CorrectionHit {
                    rule_index,
                    term_id: rules[rule_index].id,
                    term: term.to_owned(),
                    from: from.to_owned(),
                    count: 1,
                });
            }
        }
        consumed = end;
    }
    result.push_str(&text[consumed..]);
    AppliedRules { text: result, hits }
}

pub fn relevant_terms(text: &str, rules: &[Rule]) -> Vec<String> {
    let mut result = Vec::new();
    for rule in rules {
        if result.len() == 32 {
            break;
        }
        if rule
            .aliases
            .iter()
            .filter(|alias| safe_alias(alias, &rule.term, rule.context.as_deref()))
            .map(String::as_str)
            .chain(std::iter::once(rule.term.as_str()))
            .any(|alias| {
                text.char_indices()
                    .any(|(i, _)| contextual_match_at(text, alias, rule.context.as_deref(), i))
            })
            && !result.contains(&rule.term)
        {
            result.push(rule.term.clone());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(term: &str, alias: &str, context: Option<&str>) -> Rule {
        Rule {
            id: 0,
            term: term.into(),
            aliases: vec![alias.into()],
            context: context.map(str::to_owned),
        }
    }

    #[test]
    fn learns_names_and_collocations_without_normal_sentence_words() {
        let terms =
            extract_terms("Please open Claude Code，接着运行 Codex API，再打开 Claude Code。");
        assert!(terms
            .iter()
            .any(|p| p.term == "Claude Code" && p.occurrences == 2));
        assert!(terms.iter().any(|p| p.term == "Codex"));
        assert!(!terms
            .iter()
            .any(|p| matches!(p.term.as_str(), "Please" | "open")));
    }

    #[test]
    fn proposes_mixed_language_correction_with_context() {
        let proposals = correction_candidates(
            "请在终端使用克劳德 Code运行项目",
            "请在终端使用克劳德 Code运行项目",
            "请在终端使用Claude Code运行项目",
        );
        let p = proposals
            .iter()
            .find(|p| p.replacement_from.is_some())
            .expect("alias proposal");
        assert_eq!(p.term, "Claude");
        assert_eq!(p.replacement_from.as_deref(), Some("克劳德"));
        assert!(p
            .context
            .as_ref()
            .is_some_and(|s| s.contains("终端") || s.contains("Code")));
    }

    #[test]
    fn separates_cleanup_correction_from_asr_correction() {
        let proposals = correction_candidates(
            "请使用Claude Code来运行",
            "请使用Cloud Code来运行",
            "请使用Claude Code来运行",
        );
        assert!(proposals
            .iter()
            .any(|p| p.source == "cleanup_correction"
                && p.replacement_from.as_deref() == Some("Cloud")));
    }

    #[test]
    fn does_not_learn_numbers_negation_or_common_words() {
        for (from, to) in [
            ("周三", "周四"),
            ("不要", "需要"),
            ("可能", "确定"),
            ("整理", "总结"),
            ("16", "32"),
        ] {
            let a = format!("我们在项目中安排{from}执行这个步骤");
            let b = format!("我们在项目中安排{to}执行这个步骤");
            assert!(
                !correction_candidates(&a, &a, &b)
                    .iter()
                    .any(|p| p.replacement_from.is_some()),
                "{from} -> {to}"
            );
        }
    }

    #[test]
    fn ordinary_english_edit_is_not_a_terminology_rule() {
        assert!(!correction_candidates(
            "we should keep the project here",
            "we should keep the project here",
            "we should save the project here"
        )
        .iter()
        .any(|p| p.replacement_from.is_some()));
    }

    #[test]
    fn contextual_chinese_domain_phrase_can_be_learned() {
        let proposals = correction_candidates(
            "这个项目采用深度学系模型处理",
            "这个项目采用深度学系模型处理",
            "这个项目采用深度学习模型处理",
        );
        let proposal = proposals
            .iter()
            .find(|p| p.replacement_from.as_deref() == Some("深度学系模型"))
            .expect("whole Chinese domain term");
        assert_eq!(proposal.term, "深度学习模型");
        assert!(proposal.context.is_some());
        let rules = [rule(
            &proposal.term,
            "深度学系模型",
            proposal.context.as_deref(),
        )];
        assert_eq!(
            apply_rules("这个项目采用深度学系模型处理", &rules),
            "这个项目采用深度学习模型处理"
        );
        assert_eq!(
            apply_rules("另一处深度学系模型", &rules),
            "另一处深度学系模型"
        );
    }

    #[test]
    fn large_rewrite_and_size_limit_do_not_create_aliases() {
        assert!(!correction_candidates(
            "请你看一下今天准备的内容",
            "请你看一下今天准备的内容",
            "明天我们应当重新设计另一个产品方向"
        )
        .iter()
        .any(|p| p.replacement_from.is_some()));
        assert!(correction_candidates(&"a".repeat(9000), "a", "Codex").is_empty());
    }

    #[test]
    fn confirmed_context_alias_is_required_and_not_global() {
        let rules = [rule("Claude", "克劳德", Some("终端"))];
        assert_eq!(apply_rules("在终端打开克劳德", &rules), "在终端打开Claude");
        assert_eq!(apply_rules("克劳德今天到了", &rules), "克劳德今天到了");
    }

    fn observed_claude_rule() -> Rule {
        let raw = "在终端打开克劳德并运行项目";
        let proposals = correction_candidates(raw, raw, "在终端打开Claude并运行项目");
        let proposal = proposals
            .iter()
            .find(|p| p.replacement_from.is_some())
            .expect("real correction proposal");
        rule(
            &proposal.term,
            proposal.replacement_from.as_deref().unwrap(),
            proposal.context.as_deref(),
        )
    }

    #[test]
    fn generated_context_does_not_rewrite_a_name_in_another_sentence() {
        let rules = [observed_claude_rule()];
        let input = "在终端打开项目。克劳德先生今天到访。";
        assert_eq!(apply_rules(input, &rules), input);
        assert!(relevant_terms(input, &rules).is_empty());
    }

    #[test]
    fn repeated_alias_changes_only_the_occurrence_with_local_context() {
        let rules = [observed_claude_rule()];
        let input = "在终端打开克劳德并运行项目。克劳德先生今天到访。";
        assert_eq!(
            apply_rules(input, &rules),
            "在终端打开Claude并运行项目。克劳德先生今天到访。"
        );
        assert_eq!(relevant_terms(input, &rules), vec!["Claude"]);
    }

    #[test]
    fn comma_and_paragraph_separate_unrelated_contexts() {
        let rules = [observed_claude_rule()];
        for separator in ["，", ",", ";", "；", "\n", "\r\n", "!", "？"] {
            let input = format!("在终端打开项目{separator}克劳德先生今天到访");
            assert_eq!(apply_rules(&input, &rules), input);
            assert!(relevant_terms(&input, &rules).is_empty());
        }
    }

    #[test]
    fn context_is_bounded_even_without_sentence_punctuation() {
        let rules = [observed_claude_rule()];
        let input = format!(
            "在终端打开项目{}克劳德先生今天到访",
            "这里是无关内容".repeat(10)
        );
        assert_eq!(apply_rules(&input, &rules), input);
        assert!(relevant_terms(&input, &rules).is_empty());
    }

    #[test]
    fn nearby_following_context_still_supports_confirmed_alias() {
        let rules = [rule("Claude", "克劳德", Some("Code"))];
        assert_eq!(
            apply_rules("使用克劳德 Code运行", &rules),
            "使用Claude Code运行"
        );
        assert_eq!(
            relevant_terms("使用克劳德 Code运行", &rules),
            vec!["Claude"]
        );
    }

    #[test]
    fn generated_english_context_keeps_complete_words_and_matches_original() {
        for raw in [
            "Please use claud Code for this project",
            "Please use 克劳德 Code for this project",
            "We should use claud Code for this project",
            "Implementation 克劳德 Code for this project",
            "克劳德 Framework should be used for this project",
        ] {
            let corrected = raw.replace("claud", "Claude").replace("克劳德", "Claude");
            let proposals = correction_candidates(raw, raw, &corrected);
            let proposal = proposals
                .iter()
                .find(|p| p.replacement_from.is_some())
                .expect("contextual correction");
            let rules = [rule(
                &proposal.term,
                proposal.replacement_from.as_deref().unwrap(),
                proposal.context.as_deref(),
            )];
            assert_eq!(
                apply_rules(raw, &rules),
                corrected,
                "{raw}: {:?}",
                proposal.context
            );
            assert_eq!(
                relevant_terms(raw, &rules),
                vec!["Claude"],
                "{raw}: {:?}",
                proposal.context
            );
        }
    }

    #[test]
    fn english_alias_has_word_boundaries_and_canonical_case() {
        let rules = [rule("Codex", "code x", None)];
        assert_eq!(
            apply_rules("use CODE X and codex", &rules),
            "use Codex and Codex"
        );
        assert_eq!(apply_rules("decode xyz", &rules), "decode xyz");
    }

    #[test]
    fn ambiguous_aliases_and_chained_rules_are_not_applied() {
        let rules = [
            rule("Claude", "Cloud", None),
            rule("Cloudflare", "Cloud", None),
        ];
        assert_eq!(apply_rules("open Cloud", &rules), "open Cloud");
        let rules = [rule("Bravo", "Alfa", None), rule("Charlie", "Bravo", None)];
        assert_eq!(apply_rules("Alfa", &rules), "Bravo");
    }

    #[test]
    fn protected_and_unanchored_chinese_aliases_never_apply() {
        for (term, alias, context) in [
            ("需要", "不要", Some("项目")),
            ("32", "16", Some("项目")),
            ("克劳德", "小王", None),
        ] {
            let input = format!("项目{alias}");
            assert_eq!(apply_rules(&input, &[rule(term, alias, context)]), input);
        }
    }

    #[test]
    fn only_related_confirmed_terms_are_returned() {
        let rules = [
            rule("Claude", "克劳德", Some("终端")),
            rule("Codex", "code x", None),
            rule("Photoshop", "photo shop", None),
        ];
        assert_eq!(
            relevant_terms("终端打开克劳德和code x", &rules),
            vec!["Claude", "Codex"]
        );
        assert!(relevant_terms("克劳德今天到了", &rules).is_empty());
    }

    #[test]
    fn hits_include_only_confirmed_replacements_that_changed_the_original() {
        let rules = [
            rule("Claude", "克劳德", Some("终端")),
            rule("Codex", "code x", None),
        ];
        let result = apply_rules_with_hits("终端打开克劳德和克劳德，然后使用Codex", &rules);
        assert_eq!(result.text, "终端打开Claude和Claude，然后使用Codex");
        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].rule_index, 0);
        assert_eq!(result.hits[0].term_id, 0);
        assert_eq!(result.hits[0].from, "克劳德");
        assert_eq!(result.hits[0].count, 2);
        assert!(apply_rules_with_hits("在终端打开Claude", &rules)
            .hits
            .is_empty());
        assert_eq!(relevant_terms("在终端打开Claude", &rules), vec!["Claude"]);
    }

    #[test]
    fn ambiguous_overlapping_and_unanchored_matches_do_not_report_phantom_hits() {
        let rules = [
            rule("Claude", "Cloud", None),
            rule("Cloudflare", "Cloud", None),
        ];
        assert!(apply_rules_with_hits("Cloud", &rules).hits.is_empty());
        let rules = [
            rule("Claude Code", "Cloud Code", None),
            rule("Codex", "Code", None),
        ];
        let result = apply_rules_with_hits("Cloud Code", &rules);
        assert_eq!(result.text, "Claude Code");
        assert_eq!(result.hits.len(), 1);
        assert_eq!(result.hits[0].term, "Claude Code");
        assert!(
            apply_rules_with_hits("克劳德。终端", &[rule("Claude", "克劳德", Some("终端"))])
                .hits
                .is_empty()
        );
        assert!(
            apply_rules_with_hits("decode xyz", &[rule("Codex", "code x", None)])
                .hits
                .is_empty()
        );
    }

    #[test]
    fn newer_dictation_replaces_recent_hits_including_a_no_hit_result() {
        let rules = [rule("Codex", "code x", None)];
        let mut recent = RecentCorrections::default();
        assert!(recent.hits.is_empty());
        recent.record(apply_rules_with_hits("code x", &rules).hits);
        assert_eq!(recent.hits.len(), 1);
        recent.record(apply_rules_with_hits("nothing changed", &rules).hits);
        assert!(recent.hits.is_empty());
        assert!(RecentCorrections::default().hits.is_empty());
    }

    #[test]
    fn recent_hit_storage_is_bounded_without_retaining_a_body() {
        let hits = apply_rules_with_hits("code x", &[rule("Codex", "code x", None)]).hits;
        let mut recent = RecentCorrections::default();
        recent.record(vec![hits[0].clone(); 100]);
        assert_eq!(recent.hits.len(), 64);
    }
}
