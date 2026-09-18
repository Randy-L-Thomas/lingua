//! Seq2seq MT used only when the UI sends `llm: true`. Never called from lex.

pub trait Seq2SeqMt: Send + Sync {
    fn translate_body(
        &self,
        from: &str,
        to: &str,
        body: &str,
        progress: &mut dyn FnMut(&str),
    ) -> Result<String, String>;
}

/// Marian quality collapses on a 2KB jumble. Keep chat lines short.
pub const MAX_BODY_CHARS: usize = 160;

pub fn split_body_chunks(body: &str) -> Vec<&str> {
    let body = body.trim();
    if body.is_empty() {
        return Vec::new();
    }
    if body.chars().count() <= MAX_BODY_CHARS {
        return vec![body];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    let bytes = body.as_bytes();
    while start < body.len() {
        while start < body.len() && bytes[start].is_ascii_whitespace() {
            start += 1;
        }
        if start >= body.len() {
            break;
        }
        let rest = &body[start..];
        if rest.chars().count() <= MAX_BODY_CHARS {
            chunks.push(rest);
            break;
        }
        let mut end = start;
        let mut last_space = start;
        let mut chars = 0usize;
        for (i, c) in rest.char_indices() {
            chars += 1;
            let abs = start + i;
            if c.is_whitespace() {
                last_space = abs;
            }
            if chars >= MAX_BODY_CHARS {
                end = if last_space > start {
                    last_space
                } else {
                    abs + c.len_utf8()
                };
                break;
            }
            end = abs + c.len_utf8();
        }
        if end <= start {
            break;
        }
        chunks.push(body[start..end].trim());
        start = end;
    }
    chunks.into_iter().filter(|c| !c.is_empty()).collect()
}

/// Copy `Me:` / `Them:` / `Maria:` exactly; translate only the words after the colon.
pub fn map_speaker_lines(
    src: &str,
    mut translate_body: impl FnMut(&str) -> Result<String, String>,
) -> Result<String, String> {
    let mut lines = Vec::new();
    for line in src.split('\n') {
        if line.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        let (who, body) = crate::mt_lex::split_speaker(line);
        let t = if body.is_empty() {
            String::new()
        } else {
            let mut parts = Vec::new();
            for chunk in split_body_chunks(body) {
                parts.push(translate_body(chunk)?);
            }
            parts.join(" ")
        };
        if let Some(who) = who {
            lines.push(if t.is_empty() {
                format!("{who}:")
            } else {
                format!("{who}: {t}")
            });
        } else {
            lines.push(t);
        }
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    Ok(lines.join("\n"))
}

pub fn run_seq2seq(
    engine: &dyn Seq2SeqMt,
    from: &str,
    to: &str,
    src: &str,
    progress: &mut dyn FnMut(&str),
) -> Result<String, String> {
    map_speaker_lines(src, |body| engine.translate_body(from, to, body, progress))
}

pub fn model_id(from: &str, to: &str) -> String {
    format!("opus-mt-{from}-{to}")
}

#[cfg(test)]
mod tests {
    use super::{map_speaker_lines, run_seq2seq, Seq2SeqMt};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MapEng;

    impl Seq2SeqMt for MapEng {
        fn translate_body(
            &self,
            _from: &str,
            _to: &str,
            body: &str,
            progress: &mut dyn FnMut(&str),
        ) -> Result<String, String> {
            progress("getting es-en…");
            Ok(match body {
                "hola" => "hello".into(),
                "gracias" => "thanks".into(),
                "buenas tardes" => "good afternoon".into(),
                other => format!("MT:{other}"),
            })
        }
    }

    struct CountingEng {
        bodies: AtomicUsize,
    }

    impl Seq2SeqMt for CountingEng {
        fn translate_body(
            &self,
            _from: &str,
            _to: &str,
            body: &str,
            _progress: &mut dyn FnMut(&str),
        ) -> Result<String, String> {
            self.bodies.fetch_add(1, Ordering::SeqCst);
            Ok(format!("MT:{body}"))
        }
    }

    #[test]
    fn keeps_speaker_labels_and_blank_lines() {
        let out = map_speaker_lines("Me: hola\n\nThem: gracias", |body| match body {
            "hola" => Ok("hello".into()),
            "gracias" => Ok("thanks".into()),
            other => Ok(other.to_string()),
        })
        .unwrap();
        assert_eq!(out, "Me: hello\n\nThem: thanks");
    }

    #[test]
    fn keeps_named_speaker() {
        let out = map_speaker_lines("Maria: buenas tardes", |body| {
            assert_eq!(body, "buenas tardes");
            Ok("good afternoon".into())
        })
        .unwrap();
        assert_eq!(out, "Maria: good afternoon");
    }

    #[test]
    fn run_seq2seq_does_not_feed_labels_to_engine() {
        let mut saw_progress = false;
        let out = run_seq2seq(
            &MapEng,
            "es",
            "en",
            "Me: hola\n\nThem: gracias",
            &mut |_| {
                saw_progress = true;
            },
        )
        .unwrap();
        assert_eq!(out, "Me: hello\n\nThem: thanks");
        assert!(saw_progress);
    }

    #[test]
    fn unlabeled_line_is_translated_as_body() {
        let eng = CountingEng {
            bodies: AtomicUsize::new(0),
        };
        let out = run_seq2seq(&eng, "es", "en", "hola", &mut |_| {}).unwrap();
        assert_eq!(out, "MT:hola");
        assert_eq!(eng.bodies.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn long_unlabeled_jumble_is_chunked_not_one_marian_sequence() {
        let src = "hola mundo ".repeat(200);
        let eng = CountingEng {
            bodies: AtomicUsize::new(0),
        };
        let out = run_seq2seq(&eng, "es", "en", src.trim(), &mut |_| {}).unwrap();
        assert!(
            eng.bodies.load(Ordering::SeqCst) > 5,
            "2KB jumble must not be one Marian sequence, got {} calls",
            eng.bodies.load(Ordering::SeqCst)
        );
        assert!(out.contains("MT:"));
    }

    #[test]
    fn labeled_chat_is_one_body_per_line() {
        let src = "Me: hola\n\nThem: buenas tardes\n\nMe: gracias";
        let eng = CountingEng {
            bodies: AtomicUsize::new(0),
        };
        let out = run_seq2seq(&eng, "es", "en", src, &mut |_| {}).unwrap();
        assert_eq!(eng.bodies.load(Ordering::SeqCst), 3);
        assert_eq!(out.matches("Me:").count(), 2);
        assert_eq!(out.matches("Them:").count(), 1);
        assert!(out.contains("Me: MT:hola"));
        assert!(out.contains("Them: MT:buenas tardes"));
    }

    #[test]
    fn speaker_label_stays_once_when_body_is_chunked() {
        let src = format!("Me: {}", "buenas tardes ".repeat(80).trim());
        let eng = CountingEng {
            bodies: AtomicUsize::new(0),
        };
        let out = run_seq2seq(&eng, "es", "en", &src, &mut |_| {}).unwrap();
        assert!(eng.bodies.load(Ordering::SeqCst) > 1);
        assert!(out.starts_with("Me: "));
        assert_eq!(out.matches("Me:").count(), 1);
    }
}
