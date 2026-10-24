//! challenge implementation, split from the parent subsystem.

use super::*;

pub(super) fn likeness(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).filter(|(a, b)| a == b).count()
}

pub(super) fn fresh(room: String, device: String, attempts: u8) -> Challenge {
    let mut words: Vec<_> = WORDS.iter().map(|w| w.to_string()).collect();
    for i in (1..words.len()).rev() {
        let j = rand::random_range(0..=i);
        words.swap(i, j);
    }
    let answer = words[rand::random_range(0..words.len())].clone();
    Challenge {
        room,
        device,
        words,
        answer,
        remaining: attempts,
        guesses: vec![],
        locked_until: 0,
        solved: false,
    }
}

pub(super) fn display(c: &Challenge) -> String {
    if c.solved {
        return "Access granted. Use hack terminal to apply the override again.".into();
    }
    let mut out = format!(
        "Terminal challenge: {} attempts left.\nCodes: {}\nUse hack try WORD. Feedback counts letters in the correct position (0 to 5).",
        c.remaining,
        c.words.join(", ")
    );
    for (guess, score) in &c.guesses {
        out.push_str(&format!("\n{guess}: {score}/5"));
    }
    if c.locked_until > now() {
        out.push_str(&format!(
            "\nLocked out for {} seconds.",
            c.locked_until.saturating_sub(now())
        ));
    }
    out
}
