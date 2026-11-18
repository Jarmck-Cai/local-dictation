//! Pure text safety and multi-line list formatting policy shared with desktop guardrails.
//!
//! Preserves original facts, numbers, signs, percentages, dates, pronouns,
//! negations, conditionals, qualifiers, and technical tokens, while allowing
//! spoken Chinese/English enumerations ("第一/第二/第三", "首先/其次/最后", etc.)
//! to be formatted into sequential line-start ordered lists (`1. / 2. / 3.`).

/// Normalizes multi-line dictation output while preserving paragraph breaks and
/// ordered/unordered list newlines.
///
/// - Unifies `\r\n` and `\r` to `\n`
/// - Collapses horizontal whitespace and control characters within each line to single `' '`
/// - Trims leading/trailing horizontal spaces per line
/// - Caps consecutive blank lines at one (`\n\n`)
/// - Never appends a trailing newline to single-line or multi-line text
pub fn normalize_output(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut pending_newlines: usize = 0;
    let mut pending_space = false;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                let _ = chars.next();
            }
            if !normalized.is_empty() {
                pending_newlines = pending_newlines.saturating_add(1);
            }
            pending_space = false;
        } else if ch == '\n' {
            if !normalized.is_empty() {
                pending_newlines = pending_newlines.saturating_add(1);
            }
            pending_space = false;
        } else if ch.is_control() || ch.is_whitespace() {
            if !normalized.is_empty() && pending_newlines == 0 {
                pending_space = true;
            }
        } else {
            if pending_newlines > 0 {
                for _ in 0..pending_newlines.min(2) {
                    normalized.push('\n');
                }
                pending_newlines = 0;
                pending_space = false;
            } else if pending_space {
                normalized.push(' ');
                pending_space = false;
            }
            normalized.push(ch);
        }
    }

    normalized
}

pub fn validate_cleanup_text(raw: &str, candidate: &str) -> Result<String, &'static str> {
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
    Ok(normalize_output(text))
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

    // Pure spoken words ("第一/第二", "首先/其次") carry no Arabic digits in `raw`.
    if raw_numbers == numeric_fragments(&stripped_candidate) {
        return true;
    }

    // Also allow mixed ASR transcripts where some ordinals were transcribed as
    // `第1点` / `第 1，` and others as `第二点`.
    let stripped_raw = strip_arabic_spoken_ordinals(raw, max_item);
    numeric_fragments(&stripped_raw) == numeric_fragments(&stripped_candidate)
}

/// Strips contiguous `1. `, `2. `, ... (`k >= 2`, `k <= 20`) markers from line starts.
/// Returns `None` if no valid multi-item sequential ordered list was added.
fn strip_sequential_list_markers(text: &str) -> Option<(String, usize)> {
    let mut out = String::with_capacity(text.len());
    let mut expected_next: usize = 1;
    let mut current_run: usize = 0;
    let mut max_completed_run: usize = 0;
    let mut pending_first_line_restore: Option<String> = None;

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
                    pending_first_line_restore = Some(line.to_string());
                    current_run = 1;
                    expected_next = 2;
                    out.push_str(rest);
                    continue;
                }
                pending_first_line_restore = None;
                current_run = num;
                max_completed_run = max_completed_run.max(current_run);
                expected_next = num + 1;
                out.push_str(rest);
                continue;
            } else if num == 1 && current_run >= 2 {
                pending_first_line_restore = Some(line.to_string());
                current_run = 1;
                expected_next = 2;
                out.push_str(rest);
                continue;
            } else {
                // Out-of-order line-start list numbers (e.g. `1.` then `3.`) are rejected.
                return None;
            }
        }
        out.push_str(line);
    }

    if current_run == 1 || pending_first_line_restore.is_some() || max_completed_run < 2 {
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
        // Must be `1. text`, NOT a decimal like `1.5` or percentage `1.%`.
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

    let has_chinese_ordinal_pair = (contains_ordinal_one(raw) && contains_ordinal_two(raw))
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

    let has_english_ordinal_pair = has_english_word(&lower, "first")
        && has_english_word(&lower, "second")
        || has_english_word(&lower, "firstly") && has_english_word(&lower, "secondly");

    has_chinese_ordinal_pair || has_english_ordinal_pair
}

fn contains_ordinal_one(raw: &str) -> bool {
    // Exclude fixed non-enumeration compounds like "第一时间", "第一名", "第一人称", "第一线", "第一季度".
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

pub fn cleanup_budget_ms(raw: &str, max_budget_ms: u64) -> u64 {
    let length = raw.chars().filter(|c| !c.is_whitespace()).count() as u64;
    let dynamic = 3_000 + length.saturating_sub(600).saturating_mul(10).min(5_000);
    dynamic.min(max_budget_ms.max(1_000))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_output_preserves_multiline_lists_without_trailing_newlines() {
        assert_eq!(
            normalize_output("  请打开 Claude Code\t运行测试 \r\n "),
            "请打开 Claude Code 运行测试"
        );
        assert_eq!(
            normalize_output(
                "三个步骤：\r\n  1. 接入网关  \r\n  2. 同步词表 \n\n\n  3. 本地懒加载 \r\n"
            ),
            "三个步骤：\n1. 接入网关\n2. 同步词表\n\n3. 本地懒加载"
        );
    }

    #[test]
    fn allows_spoken_chinese_and_english_ordinals_rewritten_as_numbered_lists() {
        assert_eq!(
            validate_cleanup_text(
                "今天做三件事 第一 把网关鉴权写好 第二 接入 SQLite 词表同步 第三 验证离线兜底",
                "今天做三件事：\n1. 把网关鉴权写好\n2. 接入 SQLite 词表同步\n3. 验证离线兜底"
            ),
            Ok(
                "今天做三件事：\n1. 把网关鉴权写好\n2. 接入 SQLite 词表同步\n3. 验证离线兜底"
                    .into()
            )
        );

        assert_eq!(
            validate_cleanup_text(
                "首先把超时从 3000 毫秒改成 8000 毫秒，其次误差控制在 -3.5% 以内，最后在 11 月 20 日上线",
                "1. 把超时从 3000 毫秒改成 8000 毫秒；\n2. 误差控制在 -3.5% 以内；\n3. 在 11 月 20 日上线。"
            ),
            Ok("1. 把超时从 3000 毫秒改成 8000 毫秒；\n2. 误差控制在 -3.5% 以内；\n3. 在 11 月 20 日上线。".into())
        );

        assert!(validate_cleanup_text(
            "第 1 点检查端口 8787，第二点检查缓存",
            "1. 检查端口 8787\n2. 检查缓存"
        )
        .is_ok());

        assert!(validate_cleanup_text(
            "Two tasks today, first refactor the parser, second add unit tests",
            "Two tasks today:\n1. Refactor the parser\n2. Add unit tests"
        )
        .is_ok());
    }

    #[test]
    fn still_rejects_tampered_body_numbers_signs_dates_or_fake_lists() {
        // Tampered amount/port inside a spoken list
        assert_eq!(
            validate_cleanup_text(
                "第一把端口改成 8787，第二把超时设为 5 秒",
                "1. 把端口改成 8080\n2. 把超时设为 5 秒"
            ),
            Err("digits_changed")
        );
        // Dropped minus sign inside a spoken list
        assert_eq!(
            validate_cleanup_text(
                "首先偏差控制在 -3.5%，其次保留 100 元预算",
                "1. 偏差控制在 3.5%\n2. 保留 100 元预算"
            ),
            Err("digits_changed")
        );
        // Non-enumeration compound "第一时间" / "第一名" must not excuse hallucinated list numbers
        assert_eq!(
            validate_cleanup_text(
                "请第一时间通知我，争取拿到第一名",
                "1. 请通知我\n2. 争取拿到名次"
            ),
            Err("digits_changed")
        );
        // Absorbing real time numbers into list numbers must fail
        assert_eq!(
            validate_cleanup_text(
                "请在 1 点钟发邮件，第二点讨论排期",
                "1. 发邮件\n2. 讨论排期"
            ),
            Err("digits_changed")
        );
        // Non-sequential list numbers must fail
        assert_eq!(
            validate_cleanup_text("第一检查配置，第二运行测试", "1. 检查配置\n3. 运行测试"),
            Err("digits_changed")
        );
    }
}
