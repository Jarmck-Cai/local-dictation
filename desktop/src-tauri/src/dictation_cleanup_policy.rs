//! Pure text safety policy; kept free of platform and network dependencies.

pub(crate) fn validate_cleanup_text(raw: &str, candidate: &str) -> Result<String, &'static str> {
    let text = candidate.trim();
    if text.is_empty()
        || text.contains("<think>")
        || text.contains("</think>")
        || text.starts_with("```")
        || (text.starts_with(['"', '“']) && !raw.trim_start().starts_with(['"', '“']))
        || (text.starts_with(['{', '[']) && !raw.trim_start().starts_with(['{', '[']))
    {
        return Err("empty_or_unexpected_response");
    }
    if text.chars().any(|c| {
        (c.is_control() && !matches!(c, '\r' | '\n' | '\t'))
            || matches!(c, '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}')
    }) {
        return Err("unexpected_controls");
    }
    if !numbers_preserved_or_valid_spoken_list(raw, text) {
        return Err("digits_changed");
    }
    if has_added_preface(raw, text) {
        return Err("unexpected_preface");
    }
    // Ignore punctuation and paragraph whitespace when bounding additions.
    // This is a conservative expansion guard, not a claim to prove semantic
    // equivalence; factual and action fidelity also remain prompt constraints.
    let source_length = comparison_text(raw).chars().count();
    let candidate_length = comparison_text(text).chars().count();
    if candidate_length > source_length.saturating_mul(6) / 5 + 8 {
        return Err("unexpected_expansion");
    }
    if meaning_markers(raw) != meaning_markers(text) {
        return Err("meaning_markers_changed");
    }
    if qualified_markers(raw) != qualified_markers(text) {
        return Err("qualifiers_changed");
    }
    if technical_tokens(raw) != technical_tokens(text) {
        return Err("technical_tokens_changed");
    }
    Ok(text.to_string())
}

fn numeric_fragments(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut fragments = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if !chars[index].is_numeric() {
            index += 1;
            continue;
        }
        let mut start = index;
        while start > 0 && chars[start - 1].is_whitespace() {
            start -= 1;
        }
        if start > 0
            && matches!(
                chars[start - 1],
                '+' | '-' | '−' | '±' | '<' | '>' | '=' | '≤' | '≥'
            )
        {
            while start > 0
                && matches!(
                    chars[start - 1],
                    '+' | '-' | '−' | '±' | '<' | '>' | '=' | '≤' | '≥'
                )
            {
                start -= 1;
            }
        } else {
            start = index;
        }
        index += 1;
        while index < chars.len() {
            if chars[index].is_numeric() {
                index += 1;
            } else if matches!(chars[index], '.' | ',')
                && chars.get(index + 1).is_some_and(|c| c.is_numeric())
            {
                index += 2;
            } else {
                break;
            }
        }
        let mut end = index;
        while end < chars.len() && chars[end].is_whitespace() {
            end += 1;
        }
        if end < chars.len() && matches!(chars[end], '%' | '％') {
            index = end + 1;
        }
        fragments.push(
            chars[start..index]
                .iter()
                .filter(|c| !c.is_whitespace())
                .map(|c| match c {
                    '−' => '-',
                    '％' => '%',
                    other => *other,
                })
                .collect(),
        );
    }
    fragments
}

fn numbers_preserved_or_valid_spoken_list(raw: &str, candidate: &str) -> bool {
    let raw_numbers = numeric_fragments(raw);
    let candidate_numbers = numeric_fragments(candidate);
    if raw_numbers == candidate_numbers {
        return true;
    }

    let Some((stripped_candidate, max_item)) = strip_sequential_list_markers(candidate) else {
        return false;
    };
    if !has_spoken_enumeration_cues(raw, max_item) {
        return false;
    }

    if raw_numbers == numeric_fragments(&stripped_candidate) {
        return true;
    }

    // Support mixed ASR output where one ordinal was transcribed as `第 1 点`
    // and the following item as `第二点`.
    let stripped_raw = strip_arabic_spoken_ordinals(raw, max_item);
    numeric_fragments(&stripped_raw) == numeric_fragments(&stripped_candidate)
}

fn strip_sequential_list_markers(text: &str) -> Option<(String, usize)> {
    let mut out = String::with_capacity(text.len());
    let mut expected_next: usize = 1;
    let mut current_run: usize = 0;
    let mut max_completed_run: usize = 0;
    let mut pending_single_item = false;

    for (line_idx, line) in text.lines().enumerate() {
        if line_idx > 0 {
            out.push('\n');
        }
        if let Some((num, rest)) = parse_ordered_list_line(line) {
            if num == expected_next && expected_next <= 20 {
                if num == 1 {
                    if current_run == 1 {
                        return None;
                    }
                    pending_single_item = true;
                    current_run = 1;
                    expected_next = 2;
                    out.push_str(rest);
                    continue;
                }
                pending_single_item = false;
                current_run = num;
                max_completed_run = max_completed_run.max(current_run);
                expected_next = num + 1;
                out.push_str(rest);
                continue;
            } else if num == 1 && current_run >= 2 {
                pending_single_item = true;
                current_run = 1;
                expected_next = 2;
                out.push_str(rest);
                continue;
            } else {
                return None;
            }
        }
        out.push_str(line);
    }

    if current_run == 1 || pending_single_item || max_completed_run < 2 {
        return None;
    }

    Some((out, max_completed_run))
}

fn parse_ordered_list_line(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim_start_matches([' ', '\t']);
    let digit_len = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    if digit_len == 0 || digit_len > 2 {
        return None;
    }
    let number: usize = trimmed[..digit_len].parse().ok()?;
    if number == 0 {
        return None;
    }
    let after_digits = &trimmed[digit_len..];
    let rest = if let Some(r) = after_digits.strip_prefix('.') {
        if !r.starts_with([' ', '\t']) {
            return None;
        }
        r.trim_start_matches([' ', '\t'])
    } else if let Some(r) = after_digits.strip_prefix('、') {
        r.trim_start_matches([' ', '\t'])
    } else if let Some(r) = after_digits.strip_prefix(')') {
        if !r.starts_with([' ', '\t']) {
            return None;
        }
        r.trim_start_matches([' ', '\t'])
    } else {
        return None;
    };
    if rest.trim().is_empty() {
        return None;
    }
    Some((number, rest))
}

fn has_spoken_enumeration_cues(raw: &str, max_item: usize) -> bool {
    if max_item < 2 {
        return false;
    }
    let lower = raw.to_lowercase();
    let has_chinese_pair = (contains_ordinal_one(raw) && contains_ordinal_two(raw))
        || (raw.contains("首先")
            && [
                "其次", "再者", "再次", "另外", "此外", "然后", "最后", "第二",
            ]
            .iter()
            .any(|cue| raw.contains(cue)))
        || (raw.contains("一是") && raw.contains("二是"))
        || (raw.contains("其一") && raw.contains("其二"))
        || (raw.contains("一、") && raw.contains("二、"))
        || (raw.contains("一，") && raw.contains("二，") && raw.contains("点"));

    let has_english_pair = (has_english_word(&lower, "first")
        && has_english_word(&lower, "second"))
        || (has_english_word(&lower, "firstly") && has_english_word(&lower, "secondly"));

    has_chinese_pair || has_english_pair
}

fn contains_ordinal_one(raw: &str) -> bool {
    const NON_LIST_SUFFIXES: &[&str] = &[
        "时间",
        "名",
        "人称",
        "线",
        "季度",
        "届",
        "页",
        "版",
        "章",
        "节",
        "天",
        "周",
        "年",
        "个月",
        "阶段",
        "印象",
        "反应",
        "顺位",
        "梯队",
        "现场",
        "生产力",
        "要务",
    ];
    for (idx, _) in raw.match_indices("第一") {
        let after = &raw[idx + "第一".len()..];
        if !NON_LIST_SUFFIXES.iter().any(|s| after.starts_with(s)) {
            return true;
        }
    }
    contains_arabic_ordinal(raw, 1)
}

fn contains_ordinal_two(raw: &str) -> bool {
    const NON_LIST_SUFFIXES: &[&str] = &[
        "名", "人称", "线", "季度", "届", "页", "版", "章", "节", "天", "周", "年", "个月", "阶段",
        "梯队", "现场",
    ];
    for (idx, _) in raw.match_indices("第二") {
        let after = &raw[idx + "第二".len()..];
        if !NON_LIST_SUFFIXES.iter().any(|s| after.starts_with(s)) {
            return true;
        }
    }
    contains_arabic_ordinal(raw, 2)
}

fn contains_arabic_ordinal(raw: &str, target: usize) -> bool {
    for (idx, _) in raw.match_indices('第') {
        let after = raw[idx + '第'.len_utf8()..].trim_start_matches(' ');
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            continue;
        }
        if after[..digits].parse::<usize>().ok() == Some(target) {
            let suffix = after[digits..].trim_start_matches(' ');
            if suffix.is_empty()
                || [
                    "点", "个", "项", "条", "步", "是", "，", ",", "、", "：", ":",
                ]
                .iter()
                .any(|s| suffix.starts_with(s))
            {
                return true;
            }
        }
    }
    false
}

fn strip_arabic_spoken_ordinals(raw: &str, max_item: usize) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(pos) = rest.find('第') {
        out.push_str(&rest[..pos]);
        let after_di = &rest[pos + '第'.len_utf8()..];
        let trimmed = after_di.trim_start_matches(' ');
        let space_len = after_di.len() - trimmed.len();
        let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 {
            if let Ok(num) = trimmed[..digits].parse::<usize>() {
                let after_num = &trimmed[digits..];
                let after_num_trimmed = after_num.trim_start_matches(' ');
                if (1..=max_item).contains(&num)
                    && [
                        "点", "个", "项", "条", "步", "是", "，", ",", "、", "：", ":",
                    ]
                    .iter()
                    .any(|s| after_num_trimmed.starts_with(s))
                {
                    out.push('第');
                    rest = &after_di[space_len + digits..];
                    continue;
                }
            }
        }
        out.push('第');
        rest = after_di;
    }
    out.push_str(rest);
    out
}

fn has_english_word(lower: &str, target: &str) -> bool {
    lower
        .split(|c: char| !c.is_ascii_alphabetic())
        .any(|w| w == target)
}

fn comparison_text(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn has_added_preface(raw: &str, text: &str) -> bool {
    let line = text.lines().next().unwrap_or_default();
    let normalized = comparison_text(line);
    if !normalized.is_empty() && comparison_text(raw).contains(&normalized) {
        return false;
    }
    let normalized = normalized.strip_prefix("好的").unwrap_or(&normalized);
    let chinese = [
        "整理后的文字",
        "整理后的文本",
        "整理后的转录",
        "转录内容如下",
        "转写结果",
        "整理结果",
        "修正后的文本",
        "处理后的文字",
        "以下是整理",
        "下面是整理",
    ];
    if chinese.iter().any(|prefix| normalized.starts_with(prefix)) {
        return true;
    }
    let introductory = [
        "好的我会",
        "好的我将",
        "我会",
        "我将",
        "以下",
        "下面",
        "这是",
    ];
    if introductory
        .iter()
        .any(|prefix| normalized.starts_with(prefix))
        && ["转录", "整理", "润色", "录音内容"]
            .iter()
            .any(|word| normalized.contains(word))
    {
        return true;
    }
    let english = normalized.trim_start_matches("sure");
    [
        "heresthecleaned",
        "hereisthecleaned",
        "hereisthecorrected",
        "heresthecorrected",
        "hereisthepolished",
        "thecleanedtranscript",
        "cleanedtranscript",
        "correctedtranscript",
        "polishedtranscript",
        "thetranscriptis",
        "thetranscriptionis",
        "hereisthetranscript",
        "heresthetranscript",
        "hereisthetext",
        "transcript",
        "transcription",
        "cleanedtext",
        "correctedtext",
        "iwillcleanupthe",
        "illcleanupthe",
    ]
    .iter()
    .any(|prefix| english.starts_with(prefix))
}

fn meaning_markers(text: &str) -> Vec<String> {
    const WORDS: &[&str] = &[
        "i",
        "me",
        "my",
        "mine",
        "myself",
        "we",
        "us",
        "our",
        "ours",
        "ourselves",
        "you",
        "your",
        "yours",
        "yourself",
        "yourselves",
        "he",
        "him",
        "his",
        "himself",
        "she",
        "her",
        "hers",
        "herself",
        "it",
        "its",
        "itself",
        "they",
        "them",
        "their",
        "theirs",
        "themselves",
        "not",
        "no",
        "never",
        "cannot",
        "can't",
        "don't",
        "doesn't",
        "didn't",
        "won't",
        "wouldn't",
        "shouldn't",
        "couldn't",
        "mustn't",
        "isn't",
        "aren't",
        "wasn't",
        "weren't",
        "hasn't",
        "haven't",
        "hadn't",
        "shan't",
        "ain't",
    ];
    let normalized = text.to_lowercase().replace('’', "'");
    let append_word = |markers: &mut Vec<String>, word: &str| {
        if WORDS.contains(&word) {
            markers.push(word.to_string());
        } else if let Some((person, suffix)) = word.split_once('\'') {
            if ["i", "we", "you", "he", "she", "it", "they"].contains(&person)
                && ["m", "re", "s", "ll", "ve", "d"].contains(&suffix)
            {
                markers.push(person.to_string());
            }
        }
    };
    // Keep markers in spoken order: equal counts alone miss a negation moved
    // from one person's action to another person's action.
    let mut markers = Vec::new();
    let mut word = String::new();
    for c in normalized.chars() {
        if c.is_ascii_alphabetic() || c == '\'' {
            word.push(c);
        } else {
            append_word(&mut markers, &word);
            word.clear();
            if "我你您他她它咱不没未无别莫勿".contains(c) {
                markers.push(c.to_string());
            }
        }
    }
    append_word(&mut markers, &word);
    markers
}

fn qualified_markers(text: &str) -> Vec<String> {
    const ENGLISH: &[&str] = &[
        "may",
        "might",
        "maybe",
        "perhaps",
        "probably",
        "possibly",
        "certainly",
        "definitely",
        "if",
        "unless",
        "only",
        "must",
        "should",
        "could",
        "would",
    ];
    const CHINESE: &[&str] = &[
        "可能", "也许", "或许", "大概", "应该", "确定", "肯定", "如果", "除非", "只有", "否则",
    ];
    let text = text.to_lowercase();
    let mut markers = Vec::new();
    let mut offset = 0;
    while offset < text.len() {
        let suffix = &text[offset..];
        if let Some(marker) = CHINESE.iter().find(|marker| suffix.starts_with(**marker)) {
            markers.push(marker.to_string());
            offset += marker.len();
        } else if suffix.as_bytes()[0].is_ascii_alphabetic() {
            let length = suffix.bytes().take_while(u8::is_ascii_alphabetic).count();
            let word = &suffix[..length];
            if ENGLISH.contains(&word) {
                markers.push(word.to_string());
            }
            offset += length;
        } else {
            offset += suffix.chars().next().map(char::len_utf8).unwrap_or(1);
        }
    }
    markers
}

fn technical_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '，' | '。' | '；' | '！' | '？' | '、' | '“' | '”' | '"' | '`'
            )
    })
    .map(|word| {
        word.trim_matches(|c: char| {
            matches!(
                c,
                ',' | '.' | ';' | '!' | '?' | '(' | ')' | '（' | '）' | '：'
            )
        })
    })
    .filter(|word| {
        let flag = word.starts_with("--")
            || (word.starts_with('-')
                && word.chars().nth(1).is_some_and(|c| c.is_ascii_alphabetic()));
        flag || word.contains('\\')
            || word.contains('/')
            || word.contains('=')
            || word.contains('_')
    })
    .map(str::to_string)
    .collect()
}

pub(crate) fn local_endpoint(base: &str) -> Result<String, &'static str> {
    let invalid = "local_endpoint_must_be_loopback";
    let base = base.trim().trim_end_matches('/');
    let (scheme, remainder) = base.split_once("://").ok_or(invalid)?;
    if !matches!(scheme, "http" | "https")
        || remainder
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '@' | '?' | '#' | '\\'))
    {
        return Err(invalid);
    }
    let authority = remainder.split('/').next().ok_or(invalid)?;
    let port = if let Some(suffix) = authority.strip_prefix("[::1]") {
        suffix
    } else {
        let (host, port) = authority
            .split_once(':')
            .map_or((authority, ""), |(host, _port)| {
                (host, &authority[host.len()..])
            });
        if !matches!(host, "localhost" | "127.0.0.1") {
            return Err(invalid);
        }
        port
    };
    if !port.is_empty() {
        let port = port.strip_prefix(':').ok_or(invalid)?;
        if port.is_empty()
            || !port.chars().all(|c| c.is_ascii_digit())
            || port.parse::<u16>().ok().filter(|port| *port > 0).is_none()
        {
            return Err(invalid);
        }
    }
    Ok(format!("{base}/chat/completions"))
}

pub(crate) fn validation_key_from_text(text: &str) -> Result<String, &'static str> {
    for line in text.lines() {
        let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
        if let Some((name, value)) = line.split_once('=') {
            if name.trim() == "ANTHROPIC_API_KEY" {
                let value = value.trim();
                let value = if value.len() >= 2
                    && ((value.starts_with('"') && value.ends_with('"'))
                        || (value.starts_with('\'') && value.ends_with('\'')))
                {
                    &value[1..value.len() - 1]
                } else {
                    value
                };
                if !value.is_empty() && !value.chars().any(char::is_control) {
                    return Ok(value.to_string());
                }
            }
        }
    }
    Err("validation_key_not_found")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_expansion_but_accepts_natural_paragraph_breaks() {
        assert_eq!(
            validate_cleanup_text(
                "先讨论",
                "先讨论。接下来制定实施计划并完成所有功能，然后进行部署。"
            ),
            Err("unexpected_expansion")
        );
        assert!(validate_cleanup_text(
            "请先看当前问题 然后讨论下一步的安排",
            "请先看当前问题。\n\n然后讨论下一步的安排。"
        )
        .is_ok());
    }

    #[test]
    fn certainty_and_conditions_are_not_filler_words() {
        for (raw, candidate) in [
            ("可能可以先讨论", "可以先讨论。"),
            ("如果通过再开始", "通过再开始。"),
            ("we might need this", "we need this."),
            ("only if it works", "if it works."),
        ] {
            assert_eq!(
                validate_cleanup_text(raw, candidate),
                Err("qualifiers_changed")
            );
        }
    }

    #[test]
    fn accepts_conservative_punctuation_and_mixed_language() {
        assert_eq!(
            validate_cleanup_text(
                "请打开 YouTube 然后 mute OK",
                "请打开 YouTube，然后 mute，OK。"
            ),
            Ok("请打开 YouTube，然后 mute，OK。".into())
        );
    }

    #[test]
    fn rejects_empty_reasoning_and_markdown_wrappers() {
        for text in [
            "",
            " \n\t",
            "<think>secret</think>正文",
            "```text\n正文\n```",
        ] {
            assert!(validate_cleanup_text("正文", text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn preserves_ordered_numbers_signs_and_percentages() {
        for (raw, candidate) in [
            ("预算 -3.5% 到 +4", "预算 3.5% 到 +4"),
            ("参数 1,024 和 0.02", "参数 1024 和 0.02"),
            ("先 3 后 5", "先 5 后 3"),
        ] {
            assert_eq!(validate_cleanup_text(raw, candidate), Err("digits_changed"));
        }
        assert!(validate_cleanup_text("下降 −3.5 ％", "下降 -3.5%。").is_ok());
    }

    #[test]
    fn rejects_added_prefaces_but_preserves_spoken_preface() {
        for text in [
            "好的，我会按照保守整理的方式处理这段转录内容：\n原文。",
            "整理后的文本如下：\n原文。",
            "Sure, here is the cleaned transcript:\nOriginal.",
        ] {
            assert_eq!(
                validate_cleanup_text("原文 Original", text),
                Err("unexpected_preface"),
                "{text}"
            );
        }
        assert!(validate_cleanup_text(
            "好的我会整理这段转录内容 然后发送",
            "好的，我会整理这段转录内容：然后发送。"
        )
        .is_ok());
    }

    #[test]
    fn rejects_changed_person_or_negation() {
        for (raw, candidate) in [
            ("I don't need your help", "You don't need my help."),
            ("我们不要删除文件", "我们要删除文件。"),
            ("They cannot enable it", "They can enable it."),
        ] {
            assert_eq!(
                validate_cleanup_text(raw, candidate),
                Err("meaning_markers_changed")
            );
        }
        assert!(validate_cleanup_text("I don't need your help", "I don't need your help.").is_ok());
    }

    #[test]
    fn preserves_contracted_person_and_signed_thresholds() {
        assert_eq!(
            validate_cleanup_text("I'm ready", "You're ready."),
            Err("meaning_markers_changed")
        );
        assert_eq!(
            validate_cleanup_text("we’re ready", "they’re ready"),
            Err("meaning_markers_changed")
        );
        assert_eq!(
            validate_cleanup_text("误差 ≤3", "误差 ≥3"),
            Err("digits_changed")
        );
        assert_eq!(
            validate_cleanup_text("预算 ±３", "预算 ３"),
            Err("digits_changed")
        );
    }

    #[test]
    fn rejects_moving_negation_between_the_same_people() {
        assert_eq!(
            validate_cleanup_text("你不要保存，我要执行", "你要保存，我不要执行。"),
            Err("meaning_markers_changed")
        );
        assert_eq!(
            validate_cleanup_text("I do not want you to save it", "I want you to not save it."),
            Err("meaning_markers_changed")
        );
    }

    #[test]
    fn rejects_additional_common_prefaces_and_wrapped_body() {
        for candidate in [
            "好的，以下是整理后的内容：\n正文",
            "Here's the transcript:\n正文",
            "\"正文。\"",
        ] {
            assert!(
                validate_cleanup_text("正文", candidate).is_err(),
                "{candidate}"
            );
        }
    }

    #[test]
    fn rejects_added_json_wrappers_and_short_result_labels() {
        for candidate in [
            "{\"text\":\"正文\"}",
            "[正文]",
            "整理结果：正文",
            "Transcript: 正文",
        ] {
            assert!(
                validate_cleanup_text("正文", candidate).is_err(),
                "{candidate}"
            );
        }
        assert!(validate_cleanup_text("{ foo: bar }", "{ foo: bar }").is_ok());
    }

    #[test]
    fn preserves_paths_flags_identifiers_and_values() {
        for (raw, candidate) in [
            (
                r"运行 tool --dry-run --cache_size=32",
                r"运行 tool --run --cache_size=32。",
            ),
            (r"保存到 C:\work\app.py", r"保存到 C:\works\app.py。"),
            ("设置 cache_size=32", "设置 cache_count=32。"),
            ("打开 ./src/main.rs", "打开 ./src/lib.rs。"),
        ] {
            assert_eq!(
                validate_cleanup_text(raw, candidate),
                Err("technical_tokens_changed")
            );
        }
        assert!(validate_cleanup_text(
            r"运行 tool --dry-run C:\work\app.py",
            r"运行 tool --dry-run C:\work\app.py。"
        )
        .is_ok());
    }

    #[test]
    fn rejects_new_control_characters_and_keeps_literal_paths() {
        assert_eq!(
            validate_cleanup_text("正文", "正\u{1b}文"),
            Err("unexpected_controls")
        );
        assert!(validate_cleanup_text(r"C:\new\test", "C:\\new\\test\r\n").is_ok());
    }

    #[test]
    fn local_endpoint_only_accepts_literal_loopback_without_credentials_or_queries() {
        for base in [
            "http://localhost:8080/v1",
            "http://127.0.0.1:8080/v1/",
            "https://[::1]:443/v1",
        ] {
            assert!(local_endpoint(base).is_ok(), "{base}");
        }
        assert_eq!(
            local_endpoint("http://127.0.0.1:8080/v1/"),
            Ok("http://127.0.0.1:8080/v1/chat/completions".into())
        );
        for base in [
            "https://example.com/v1",
            "http://localhost.example.com/v1",
            "http://127.0.0.1@evil.test/v1",
            "http://user@localhost/v1",
            "file://localhost/v1",
            "http://127.1/v1",
            "http://localhost/v1?redirect=evil",
            "http://[::ffff:127.0.0.1]/v1",
            "http://localhost:wrong/v1",
            "http://localhost/v1#secret",
        ] {
            assert_eq!(
                local_endpoint(base),
                Err("local_endpoint_must_be_loopback"),
                "{base}"
            );
        }
    }

    #[test]
    fn imports_only_explicit_anthropic_key_without_echoing_other_lines() {
        assert_eq!(
            validation_key_from_text("OTHER=fake\nANTHROPIC_API_KEY='fake-test-key'\n"),
            Ok("fake-test-key".into())
        );
        assert_eq!(
            validation_key_from_text("ANTHROPIC_API_KEY=\n"),
            Err("validation_key_not_found")
        );
        assert_eq!(
            validation_key_from_text("# ANTHROPIC_API_KEY=fake\nOTHER=value"),
            Err("validation_key_not_found")
        );
    }

    #[test]
    fn allows_spoken_ordinals_rewritten_to_multiline_ordered_lists() {
        // Positive 1: Spoken 第一/第二/第三 -> 1. / 2. / 3.
        assert_eq!(
            validate_cleanup_text(
                "今天有三个重点 第一 把网关鉴权写好 第二 同步 SQLite 词库 第三 支持本地懒加载兜底",
                "今天有三个重点：\n1. 把网关鉴权写好\n2. 同步 SQLite 词库\n3. 支持本地懒加载兜底"
            ),
            Ok(
                "今天有三个重点：\n1. 把网关鉴权写好\n2. 同步 SQLite 词库\n3. 支持本地懒加载兜底"
                    .into()
            )
        );

        // Positive 2: Spoken 首先/其次/最后 with inline amounts, dates, negative percentages, and negations
        assert!(validate_cleanup_text(
            "首先不要修改 3500 元预算，其次误差控制在 -2.5% 以内，最后在 11 月 20 日前交付",
            "1. 不要修改 3500 元预算；\n2. 误差控制在 -2.5% 以内；\n3. 在 11 月 20 日前交付。"
        )
        .is_ok());

        // Positive 3: Spoken 一是/二是 and 其一/其二
        assert!(validate_cleanup_text(
            "主要改动两块，一是放开多行列表，二是保持零常驻内存",
            "主要改动两块：\n1. 放开多行列表\n2. 保持零常驻内存"
        )
        .is_ok());
        assert!(validate_cleanup_text(
            "其一检查端口 8787，其二检查证书有效期 90 天",
            "1. 检查端口 8787\n2. 检查证书有效期 90 天"
        )
        .is_ok());

        // Positive 4: Mixed ASR output (`第 1 点` + `第二点`) and English (`first` + `second`)
        assert!(validate_cleanup_text(
            "第 1 点打开日志，第二点核对超时 8000 毫秒",
            "1. 打开日志\n2. 核对超时 8000 毫秒"
        )
        .is_ok());
        assert!(validate_cleanup_text(
            "Please do two things, first run cargo test, second push the branch",
            "Please do two things:\n1. Run cargo test\n2. Push the branch"
        )
        .is_ok());
    }

    #[test]
    fn rejects_tampering_inside_spoken_lists_and_fake_non_enumeration_lists() {
        // Negative 1: Tampered amount inside a valid spoken list
        assert_eq!(
            validate_cleanup_text(
                "第一预算定在 3500 元，第二工期定在 14 天",
                "1. 预算定在 4500 元\n2. 工期定在 14 天"
            ),
            Err("digits_changed")
        );

        // Negative 2: Dropped minus sign inside a spoken list
        assert_eq!(
            validate_cleanup_text(
                "首先偏差不超过 -1.5%，其次保留 2 台机器",
                "1. 偏差不超过 1.5%\n2. 保留 2 台机器"
            ),
            Err("digits_changed")
        );

        // Negative 3: Altered date inside a spoken list
        assert_eq!(
            validate_cleanup_text(
                "第一在 11 月 17 日冻结，第二在 11 月 20 日发布",
                "1. 在 11 月 18 日冻结\n2. 在 11 月 20 日发布"
            ),
            Err("digits_changed")
        );

        // Negative 4: Dropped negation inside a spoken list
        assert_eq!(
            validate_cleanup_text("第一不要删除备份，第二更新配置", "1. 删除备份\n2. 更新配置"),
            Err("meaning_markers_changed")
        );

        // Negative 5: Non-enumeration compounds ("第一时间", "第一名") must not allow list digits
        assert_eq!(
            validate_cleanup_text(
                "请第一时间同步进展，争取拿到第一名",
                "1. 请同步进展\n2. 争取拿到好成绩"
            ),
            Err("digits_changed")
        );

        // Negative 6: Absorbing real time/quantity digit ("1 点钟") into list marker `1.`
        assert_eq!(
            validate_cleanup_text(
                "请在 1 点钟发邮件，第二点确认会议室",
                "1. 发邮件\n2. 确认会议室"
            ),
            Err("digits_changed")
        );

        // Negative 7: Out-of-order / skipped list numbers (`1.` then `3.`) or single `1.`
        assert_eq!(
            validate_cleanup_text("第一检查网关，第二检查词表", "1. 检查网关\n3. 检查词表"),
            Err("digits_changed")
        );
        assert_eq!(
            validate_cleanup_text("第一检查网关", "1. 检查网关"),
            Err("digits_changed")
        );
    }
}
