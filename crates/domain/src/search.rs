//! What a search query asks for, as Postgres's full-text search with the `simple`
//! configuration runs it, and its fuzzy match of titles by trigrams.
//!
//! The syntax:
//!
//! - **Words** must all be in what is found, in any order. A word of two characters or more
//!   also finds the words it starts: `rep` finds `report`. Case is ignored.
//! - **`"Quoted words"`** must be there one after another, in that order, each whole.
//! - **`-word`** and **`-"quoted words"`** leave out what holds them.
//! - **`OR`**, in capitals, between two terms finds either.
//!
//! Punctuation splits a word where Postgres's parser splits it, so `e-mail` finds `e-mail`
//! and `3.14` finds `3.14`; a term without a letter or a digit is left out. The words, but
//! for those left out, also match titles loosely, by trigrams, so a misspelled title still
//! finds its page.
//!
//! The query given to Postgres is built here, every term quoted, so nothing a person types
//! is read as the query language's own syntax.

/// The longest query, in characters.
pub const MAX_QUERY_CHARS: usize = 200;

/// The most terms a query has, words and quoted ones together, those left out included.
pub const MAX_TERMS: usize = 16;

/// The fewest characters of letters and digits a query's words need to match titles
/// loosely: with fewer, trigrams match almost any title.
pub const MIN_FUZZY_CHARS: usize = 3;

/// A query, ready for the database.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// For `to_tsquery('simple', …)`; `None` for a query without words, which lists what the
    /// filters let through, most recently changed first.
    pub text_search: Option<String>,
    /// For the trigram match of titles: the words not left out; `None` when they are too
    /// short to match loosely.
    pub fuzzy: Option<String>,
    /// For `to_tsquery('simple', …)` too: what holds none of the words left out, which a
    /// loose match must hold as well; `None` when none are.
    pub excluded: Option<String>,
}

impl Query {
    /// A query without words.
    pub fn is_empty(&self) -> bool {
        self.text_search.is_none()
    }
}

/// Why a query cannot be run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QueryError {
    #[error("a search is at most {MAX_QUERY_CHARS} characters")]
    TooLong,
    #[error("a search has at most {MAX_TERMS} words and quoted phrases")]
    TooManyTerms,
    #[error("a search needs a word to find, not only words to leave out")]
    OnlyExcluded,
}

/// A word or quoted words.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Term {
    Word(String),
    Phrase(String),
}

impl Term {
    /// The term in the query language: quoted, a word of two characters or more as a prefix.
    fn render(&self) -> String {
        match self {
            Self::Word(word) if word.chars().count() >= 2 => format!("{}:*", quote(word)),
            Self::Word(word) | Self::Phrase(word) => quote(word),
        }
    }

    fn text(&self) -> &str {
        match self {
            Self::Word(text) | Self::Phrase(text) => text,
        }
    }
}

/// Text as a quoted lexeme of the query language, which doubles quotes and backslashes in
/// it. `to_tsquery` reads a quoted lexeme's text with the configuration's parser, so
/// punctuation in it splits it as it splits the pages' text.
fn quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('\'');
    for c in text.chars() {
        match c {
            '\'' => quoted.push_str("''"),
            '\\' => quoted.push_str("\\\\"),
            c => quoted.push(c),
        }
    }
    quoted.push('\'');
    quoted
}

/// A term as typed, before it is kept or left out.
#[derive(Debug)]
enum Token {
    Term { term: Term, excluded: bool },
    Or,
}

fn tokens(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        let excluded = c == '-'
            && chars
                .clone()
                .nth(1)
                .is_some_and(|next| !next.is_whitespace());
        if excluded {
            chars.next();
        }
        if chars.peek() == Some(&'"') {
            chars.next();
            let phrase: String = chars.by_ref().take_while(|&c| c != '"').collect();
            tokens.push(Token::Term {
                term: Term::Phrase(phrase.split_whitespace().collect::<Vec<_>>().join(" ")),
                excluded,
            });
            continue;
        }
        let mut word = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() || c == '"' {
                break;
            }
            word.push(c);
            chars.next();
        }
        if word == "OR" && !excluded {
            tokens.push(Token::Or);
        } else {
            tokens.push(Token::Term {
                term: Term::Word(word),
                excluded,
            });
        }
    }
    tokens
}

/// Whether a term finds anything: one without a letter or a digit is no lexeme.
fn finds(term: &Term) -> bool {
    term.text().chars().any(char::is_alphanumeric)
}

/// Reads a query; see the module docs.
pub fn parse(text: &str) -> Result<Query, QueryError> {
    if text.chars().count() > MAX_QUERY_CHARS {
        return Err(QueryError::TooLong);
    }
    let tokens = tokens(text);
    let terms = tokens
        .iter()
        .filter(|token| matches!(token, Token::Term { .. }))
        .count();
    if terms > MAX_TERMS {
        return Err(QueryError::TooManyTerms);
    }
    // Clauses that must all hold: each the terms either of which will do, or one term to
    // leave out. OR joins the terms beside it, and nothing else.
    let mut wanted: Vec<Vec<Term>> = Vec::new();
    let mut unwanted: Vec<Term> = Vec::new();
    let mut joining = false;
    for token in tokens {
        match token {
            Token::Or => joining = true,
            Token::Term { term, excluded } => {
                if !finds(&term) {
                    continue;
                }
                if excluded {
                    unwanted.push(term);
                    joining = false;
                } else if joining && let Some(last) = wanted.last_mut() {
                    last.push(term);
                    joining = false;
                } else {
                    wanted.push(vec![term]);
                    joining = false;
                }
            }
        }
    }
    if wanted.is_empty() {
        return if unwanted.is_empty() {
            Ok(Query::default())
        } else {
            Err(QueryError::OnlyExcluded)
        };
    }
    let mut clauses: Vec<String> = wanted
        .iter()
        .map(|either| match either.as_slice() {
            [one] => one.render(),
            many => format!(
                "({})",
                many.iter()
                    .map(Term::render)
                    .collect::<Vec<_>>()
                    .join(" | ")
            ),
        })
        .collect();
    let excluded: Vec<String> = unwanted
        .iter()
        .map(|term| format!("!{}", term.render()))
        .collect();
    clauses.extend(excluded.iter().cloned());
    let words: Vec<&str> = wanted.iter().flatten().map(Term::text).collect();
    let fuzzy = words.join(" ");
    let letters = fuzzy.chars().filter(|c| c.is_alphanumeric()).count();
    Ok(Query {
        text_search: Some(clauses.join(" & ")),
        fuzzy: (letters >= MIN_FUZZY_CHARS).then_some(fuzzy),
        excluded: (!excluded.is_empty()).then(|| excluded.join(" & ")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(text: &str) -> String {
        parse(text)
            .expect("a query")
            .text_search
            .expect("with words")
    }

    #[test]
    fn words_must_all_be_there_and_find_what_they_start() {
        assert_eq!(search("quarterly road"), "'quarterly':* & 'road':*");
        assert_eq!(
            parse("Quarterly  roadmap")
                .expect("a query")
                .fuzzy
                .as_deref(),
            Some("Quarterly roadmap")
        );
    }

    #[test]
    fn a_word_of_one_character_is_whole() {
        assert_eq!(search("a plan"), "'a' & 'plan':*");
        assert_eq!(search("ü"), "'ü'");
        // Characters, not bytes: two letters make a prefix, in any script.
        assert_eq!(search("東京"), "'東京':*");
    }

    #[test]
    fn quoted_words_are_a_phrase_of_whole_words() {
        assert_eq!(search("\"ship  it\" now"), "'ship it' & 'now':*");
        // An unclosed quote runs to the end.
        assert_eq!(search("\"ship it"), "'ship it'");
    }

    #[test]
    fn excluded_terms_leave_out_what_holds_them() {
        assert_eq!(search("plan -draft"), "'plan':* & !'draft':*");
        let query = parse("plan -draft -\"old idea\"").expect("a query");
        assert_eq!(
            query.excluded.as_deref(),
            Some("!'draft':* & !'old idea'"),
            "a loose match keeps to them too"
        );
        assert_eq!(parse("plan").expect("a query").excluded, None);
        assert_eq!(search("plan -\"old idea\""), "'plan':* & !'old idea'");
        // A dash alone, or inside a word, is no exclusion.
        assert_eq!(search("plan - e-mail"), "'plan':* & 'e-mail':*");
        assert_eq!(parse("-draft"), Err(QueryError::OnlyExcluded));
    }

    #[test]
    fn or_joins_the_terms_beside_it() {
        assert_eq!(
            search("plan OR roadmap q1"),
            "('plan':* | 'roadmap':*) & 'q1':*"
        );
        assert_eq!(search("a OR b OR c"), "('a' | 'b' | 'c')");
        // Lowercase is a word, and an OR with nothing to join is dropped.
        assert_eq!(search("this or that"), "'this':* & 'or':* & 'that':*");
        assert_eq!(search("OR plan OR"), "'plan':*");
        assert_eq!(search("plan OR -draft"), "'plan':* & !'draft':*");
    }

    #[test]
    fn nothing_typed_is_read_as_the_query_language() {
        assert_eq!(search("o'neil"), "'o''neil':*");
        assert_eq!(search("back\\slash"), "'back\\\\slash':*");
        assert_eq!(search("a&b|!c:*"), "'a&b|!c:*':*");
        // Punctuation alone finds nothing, and is left out.
        assert_eq!(search("plan !!! ..."), "'plan':*");
        assert_eq!(parse("!!! ..."), Ok(Query::default()));
    }

    #[test]
    fn an_empty_query_lists_what_the_filters_let_through() {
        assert!(parse("").expect("a query").is_empty());
        assert!(parse("   ").expect("a query").is_empty());
    }

    #[test]
    fn short_words_do_not_match_titles_loosely() {
        assert_eq!(parse("ab").expect("a query").fuzzy, None);
        assert_eq!(
            parse("abc -draft").expect("a query").fuzzy.as_deref(),
            Some("abc")
        );
    }

    #[test]
    fn long_queries_and_many_terms_are_refused() {
        assert_eq!(parse(&"a".repeat(201)), Err(QueryError::TooLong));
        assert!(parse(&"ü".repeat(200)).is_ok(), "characters, not bytes");
        let many = vec!["word"; 17].join(" ");
        assert_eq!(parse(&many), Err(QueryError::TooManyTerms));
        assert!(parse(&vec!["word"; 16].join(" ")).is_ok());
    }
}
