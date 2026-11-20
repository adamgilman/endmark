//! Property: removing the records from a file leaves the body, byte for byte
//! (RFC goal 1), and every record placed outside a code fence is found.
//!
//! Files are generated from a fixed seed, so a failure is reproducible.

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // SplitMix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a>(&mut self, pool: &[&'a str]) -> &'a str {
        pool[self.below(pool.len())]
    }
}

const PROSE: &[&str] = &[
    "",
    "A paragraph of text.",
    "# Heading",
    "- a list item",
    "  continued, indented",
    "> a quote",
    "Arrow --> and bang --!> in prose.",
    "An unclosed <!-- comment start.",
    "Inline ```code``` with three backticks.",
    "    <!--endmark indented, so not a record",
    " <!--endmark one space in, so not a record",
    "<!--endmarked is a different word",
    "Ünïcödé — 日本語.",
    "-->",
];

/// Lines that are safe inside a code fence: none of them closes one.
const CODE: &[&str] = &[
    "",
    "let x = 1;",
    "<!--endmark comment c-0 by=example at=2026-01-01T00:00:00Z -->",
    "<!--endmark suggest s-0 by=example at=2026-01-01T00:00:00Z",
    "-->",
    "graph LR; A --> B",
    "``",
    "~~",
];

const FENCES: &[(&str, &str)] = &[
    ("```", "```"),
    ("```rust", "```"),
    ("~~~", "~~~"),
    ("````", "`````"),
    ("  ```", "   ```"),
];

fn record(rng: &mut Rng, n: usize, out: &mut Vec<String>) {
    let stamp = "by=gen at=2026-11-13T09:00:00Z";
    match rng.below(3) {
        0 => out.push(format!("<!--endmark comment c-{n} {stamp} -->")),
        1 => {
            out.push(format!("<!--endmark comment c-{n} {stamp}"));
            out.push("A note with an escaped arrow --\\> in it.".to_string());
            out.push(String::new());
            out.push("```text".to_string());
            out.push("<!--endmark comment c-0 by=shown at=2026-01-01T00:00:00Z".to_string());
            out.push("```".to_string());
            out.push("-->".to_string());
        }
        _ => {
            out.push(format!("<!--endmark suggest s-{n} {stamp}"));
            out.push("```replace".to_string());
            out.push("text that is nowhere in the body".to_string());
            out.push(String::new());
            out.push("```".to_string());
            out.push("```with".to_string());
            out.push("```".to_string());
            out.push("-->".to_string());
        }
    }
}

#[test]
fn removing_records_leaves_the_body_byte_for_byte() {
    let mut rng = Rng(0x00E1_D0AA_2026);
    for case in 0..2000 {
        let mut file: Vec<String> = Vec::new();
        let mut body: Vec<String> = Vec::new();
        let mut records = 0;
        for _ in 0..rng.below(30) {
            match rng.below(6) {
                0 | 1 => {
                    records += 1;
                    record(&mut rng, records, &mut file);
                }
                2 => {
                    let (open, close) = FENCES[rng.below(FENCES.len())];
                    let mut block = vec![open.to_string()];
                    for _ in 0..rng.below(5) {
                        block.push(rng.pick(CODE).to_string());
                    }
                    block.push(close.to_string());
                    file.extend(block.iter().cloned());
                    body.extend(block);
                }
                _ => {
                    let line = rng.pick(PROSE).to_string();
                    file.push(line.clone());
                    body.push(line);
                }
            }
        }
        // Sometimes end with a newline, sometimes not, sometimes with CRLF.
        let ending = rng.below(3);
        let (mut source, mut expected) = (file.join("\n"), body.join("\n"));
        if ending > 0 && !file.is_empty() {
            source.push('\n');
            // The final newline belongs to the body unless the file is all records.
            if !body.is_empty() || file.is_empty() {
                expected.push('\n');
            }
        }
        if ending == 2 {
            source = source.replace('\n', "\r\n");
        }

        let doc = endmark_core::parse(&source);
        assert_eq!(doc.body, expected, "case {case}:\n{source}");
        assert_eq!(
            doc.items.len(),
            records,
            "case {case}: every record is an item\n{source}"
        );
        // Records are dropped at random places, so CommonMark may not hide all
        // of them. Nothing else may be wrong with a generated file.
        let unexpected: Vec<_> = doc
            .diagnostics
            .iter()
            .filter(|d| d.severity() == endmark_core::Severity::Error && d.code != endmark_core::Code::VisibleRecord)
            .collect();
        assert!(unexpected.is_empty(), "case {case}: {unexpected:?}\n{source}");
    }
}
