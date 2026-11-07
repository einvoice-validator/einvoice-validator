//! Content models compiled to Glushkov automata.
//!
//! A particle tree (sequences, choices, element declarations and wildcards, each
//! with `minOccurs`/`maxOccurs`) is expanded into a regular expression over
//! positions (one position per occurrence of a term), from which the Glushkov
//! construction gives the first positions, the follow positions of each position and
//! the final positions. The Unique Particle Attribution constraint of XML Schema
//! (cos-nonambig) is exactly the determinism of that automaton: at load, no two
//! positions reachable from the same state may compete for the same element. A
//! child element therefore matches at most one position, and validation is one
//! table lookup per child.

use crate::model::{ElementDecl, ElementId, Wildcard};

/// The largest finite `minOccurs`/`maxOccurs` accepted; larger finite bounds would
/// multiply positions and are not used by the supported schemas.
pub(crate) const MAX_FINITE_OCCURS: u32 = 64;

#[derive(Debug, Clone)]
pub(crate) enum ParticleTerm {
    Element(ElementId),
    Wildcard(Wildcard),
    Sequence(Vec<Particle>),
    Choice(Vec<Particle>),
}

#[derive(Debug, Clone)]
pub(crate) struct Particle {
    pub(crate) term: ParticleTerm,
    pub(crate) min: u32,
    /// `None` for `unbounded`.
    pub(crate) max: Option<u32>,
}

/// What a position matches.
#[derive(Debug, Clone)]
pub(crate) enum Term {
    Element(ElementId),
    Wildcard(Wildcard),
}

#[derive(Debug, Clone)]
pub(crate) struct Automaton {
    terms: Vec<Term>,
    first: Vec<usize>,
    follow: Vec<Vec<usize>>,
    is_final: Vec<bool>,
    nullable: bool,
}

/// A state: before any child (`None`) or after the child that matched a position.
pub(crate) type State = Option<usize>;

enum Re {
    Position(usize),
    Sequence(Vec<Re>),
    Choice(Vec<Re>),
    Star(Box<Re>),
    Optional(Box<Re>),
}

fn expand(particle: &Particle, terms: &mut Vec<Term>) -> Re {
    let build = |terms: &mut Vec<Term>| -> Re {
        match &particle.term {
            ParticleTerm::Element(id) => {
                terms.push(Term::Element(*id));
                Re::Position(terms.len() - 1)
            }
            ParticleTerm::Wildcard(w) => {
                terms.push(Term::Wildcard(w.clone()));
                Re::Position(terms.len() - 1)
            }
            ParticleTerm::Sequence(items) => {
                Re::Sequence(items.iter().map(|p| expand(p, terms)).collect())
            }
            ParticleTerm::Choice(items) => {
                Re::Choice(items.iter().map(|p| expand(p, terms)).collect())
            }
        }
    };
    let mut parts = Vec::new();
    for _ in 0..particle.min {
        parts.push(build(terms));
    }
    match particle.max {
        None => parts.push(Re::Star(Box::new(build(terms)))),
        Some(max) => {
            // a{min,max} = a^min (a (a (...)?)?)? : nested optionals keep the
            // expansion deterministic when the particle itself is.
            let mut optional: Option<Re> = None;
            for _ in particle.min..max {
                let copy = build(terms);
                optional = Some(Re::Optional(Box::new(match optional.take() {
                    None => copy,
                    Some(inner) => Re::Sequence(vec![copy, inner]),
                })));
            }
            parts.extend(optional);
        }
    }
    Re::Sequence(parts)
}

struct Sets {
    nullable: bool,
    first: Vec<usize>,
    last: Vec<usize>,
}

fn glushkov(re: &Re, follow: &mut [Vec<usize>]) -> Sets {
    match re {
        Re::Position(p) => Sets {
            nullable: false,
            first: vec![*p],
            last: vec![*p],
        },
        Re::Sequence(items) => {
            let mut result = Sets {
                nullable: true,
                first: Vec::new(),
                last: Vec::new(),
            };
            for item in items {
                let sets = glushkov(item, follow);
                for &p in &result.last {
                    follow[p].extend_from_slice(&sets.first);
                }
                if result.nullable {
                    result.first.extend_from_slice(&sets.first);
                }
                if sets.nullable {
                    result.last.extend_from_slice(&sets.last);
                } else {
                    result.last = sets.last;
                }
                result.nullable &= sets.nullable;
            }
            result
        }
        Re::Choice(items) => {
            let mut result = Sets {
                nullable: false,
                first: Vec::new(),
                last: Vec::new(),
            };
            for item in items {
                let sets = glushkov(item, follow);
                result.nullable |= sets.nullable;
                result.first.extend(sets.first);
                result.last.extend(sets.last);
            }
            result
        }
        Re::Star(inner) => {
            let sets = glushkov(inner, follow);
            for &p in &sets.last {
                follow[p].extend_from_slice(&sets.first);
            }
            Sets {
                nullable: true,
                ..sets
            }
        }
        Re::Optional(inner) => Sets {
            nullable: true,
            ..glushkov(inner, follow)
        },
    }
}

fn dedup(mut v: Vec<usize>) -> Vec<usize> {
    v.sort_unstable();
    v.dedup();
    v
}

impl Automaton {
    /// Compiles a particle. Fails with a description when the content model is not
    /// deterministic (Unique Particle Attribution).
    pub(crate) fn compile(particle: &Particle, elements: &[ElementDecl]) -> Result<Self, String> {
        let mut terms = Vec::new();
        let re = expand(particle, &mut terms);
        let mut follow = vec![Vec::new(); terms.len()];
        let sets = glushkov(&re, &mut follow);
        let mut is_final = vec![false; terms.len()];
        for &p in &sets.last {
            is_final[p] = true;
        }
        let automaton = Self {
            first: dedup(sets.first),
            follow: follow.into_iter().map(dedup).collect(),
            terms,
            is_final,
            nullable: sets.nullable,
        };
        automaton.check_deterministic(elements)?;
        Ok(automaton)
    }

    /// Whether the content model has no position at all (an empty content type
    /// unless the type is mixed).
    pub(crate) fn is_empty(&self) -> bool {
        self.terms.is_empty() && self.nullable
    }

    fn check_deterministic(&self, elements: &[ElementDecl]) -> Result<(), String> {
        let compete = |a: &Term, b: &Term| match (a, b) {
            (Term::Element(x), Term::Element(y)) => elements[*x].name == elements[*y].name,
            (Term::Element(e), Term::Wildcard(w)) | (Term::Wildcard(w), Term::Element(e)) => {
                w.allows(elements[*e].name.namespace.as_deref())
            }
            // The supported namespace constraints (##any, ##other) always share
            // some namespace.
            (Term::Wildcard(_), Term::Wildcard(_)) => true,
        };
        for set in std::iter::once(&self.first).chain(self.follow.iter()) {
            for (i, &a) in set.iter().enumerate() {
                for &b in &set[i + 1..] {
                    if compete(&self.terms[a], &self.terms[b]) {
                        return Err(format!(
                            "content model is not deterministic: {} and {} compete",
                            self.describe(a, elements),
                            self.describe(b, elements)
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn describe(&self, position: usize, elements: &[ElementDecl]) -> String {
        match &self.terms[position] {
            Term::Element(e) => elements[*e].name.to_string(),
            Term::Wildcard(_) => "a wildcard".to_owned(),
        }
    }

    fn successors(&self, state: State) -> &[usize] {
        match state {
            None => &self.first,
            Some(p) => &self.follow[p],
        }
    }

    /// The position a child element matches from `state`, with its term.
    pub(crate) fn step(
        &self,
        state: State,
        namespace: Option<&str>,
        local: &str,
        elements: &[ElementDecl],
    ) -> Option<(usize, &Term)> {
        self.successors(state).iter().find_map(|&p| {
            let term = &self.terms[p];
            let matches = match term {
                Term::Element(e) => elements[*e].name.matches(namespace, local),
                Term::Wildcard(w) => w.allows(namespace),
            };
            matches.then_some((p, term))
        })
    }

    pub(crate) fn accepts(&self, state: State) -> bool {
        match state {
            None => self.nullable,
            Some(p) => self.is_final[p],
        }
    }

    /// What may come next from `state`, for error messages.
    pub(crate) fn expected(&self, state: State, elements: &[ElementDecl]) -> Vec<String> {
        let mut names: Vec<String> = self
            .successors(state)
            .iter()
            .map(|&p| match &self.terms[p] {
                Term::Element(e) => elements[*e].name.to_string(),
                Term::Wildcard(w) => match &w.namespaces {
                    crate::model::NamespaceConstraint::Any => "##any".to_owned(),
                    crate::model::NamespaceConstraint::Other(Some(ns)) => {
                        format!("##other{{{ns}}}")
                    }
                    crate::model::NamespaceConstraint::Other(None) => "##other".to_owned(),
                },
            })
            .collect();
        names.dedup();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NamespaceConstraint, ProcessContents, QName};

    fn decls(names: &[&str]) -> Vec<ElementDecl> {
        names
            .iter()
            .map(|n| ElementDecl {
                name: QName::new(Some("urn:t"), n),
                type_id: 0,
            })
            .collect()
    }

    fn element(id: ElementId, min: u32, max: Option<u32>) -> Particle {
        Particle {
            term: ParticleTerm::Element(id),
            min,
            max,
        }
    }

    fn run(automaton: &Automaton, elements: &[ElementDecl], children: &[&str]) -> bool {
        let mut state = None;
        for child in children {
            match automaton.step(state, Some("urn:t"), child, elements) {
                Some((p, _)) => state = Some(p),
                None => return false,
            }
        }
        automaton.accepts(state)
    }

    #[test]
    fn sequence_with_occurrences() {
        let elements = decls(&["a", "b", "c"]);
        let model = Particle {
            term: ParticleTerm::Sequence(vec![
                element(0, 0, Some(1)),
                element(1, 1, None),
                element(2, 2, Some(3)),
            ]),
            min: 1,
            max: Some(1),
        };
        let automaton = Automaton::compile(&model, &elements).unwrap();
        assert!(run(&automaton, &elements, &["a", "b", "c", "c"]));
        assert!(run(&automaton, &elements, &["b", "b", "c", "c", "c"]));
        assert!(!run(&automaton, &elements, &["b", "c"]));
        assert!(!run(&automaton, &elements, &["b", "c", "c", "c", "c"]));
        assert!(!run(&automaton, &elements, &["a", "a", "b", "c", "c"]));
        assert!(!run(&automaton, &elements, &["c", "c"]));
    }

    #[test]
    fn repeated_choice_of_sequences() {
        let elements = decls(&["a", "b", "c"]);
        let model = Particle {
            term: ParticleTerm::Choice(vec![
                Particle {
                    term: ParticleTerm::Sequence(vec![
                        element(0, 1, Some(1)),
                        element(1, 0, Some(1)),
                    ]),
                    min: 1,
                    max: Some(1),
                },
                element(2, 1, Some(1)),
            ]),
            min: 0,
            max: None,
        };
        let automaton = Automaton::compile(&model, &elements).unwrap();
        assert!(run(&automaton, &elements, &[]));
        assert!(run(&automaton, &elements, &["a", "c", "a", "b", "c"]));
        assert!(!run(&automaton, &elements, &["b"]));
    }

    /// Every end index reachable by matching `particle` from `start` in `word`
    /// (element ids as letters): a direct backtracking reading of the particle,
    /// independent of the Glushkov construction.
    fn ends(
        particle: &Particle,
        word: &[ElementId],
        start: usize,
    ) -> std::collections::BTreeSet<usize> {
        use std::collections::BTreeSet;
        let once = |from: usize| -> BTreeSet<usize> {
            match &particle.term {
                ParticleTerm::Element(id) => word
                    .get(from)
                    .filter(|&&c| c == *id)
                    .map(|_| from + 1)
                    .into_iter()
                    .collect(),
                ParticleTerm::Wildcard(_) => word.get(from).map(|_| from + 1).into_iter().collect(),
                ParticleTerm::Sequence(items) => {
                    items
                        .iter()
                        .fold(BTreeSet::from([from]), |positions, item| {
                            positions
                                .iter()
                                .flat_map(|&p| ends(item, word, p))
                                .collect()
                        })
                }
                ParticleTerm::Choice(items) => items
                    .iter()
                    .flat_map(|item| ends(item, word, from))
                    .collect(),
            }
        };
        let mut result = BTreeSet::new();
        let mut current = BTreeSet::from([start]);
        let limit = particle
            .max
            .unwrap_or(u32::try_from(word.len()).unwrap() + particle.min + 1);
        for count in 0..=limit {
            if count >= particle.min {
                result.extend(current.iter().copied());
            }
            if count == limit {
                break;
            }
            current = current.iter().flat_map(|&p| once(p)).collect();
        }
        result
    }

    fn particle_strategy() -> impl proptest::strategy::Strategy<Value = Particle> {
        use proptest::prelude::*;
        let occurs = prop_oneof![
            Just((1_u32, Some(1_u32))),
            Just((0, Some(1))),
            Just((0, None)),
            Just((1, None)),
            Just((2, Some(3))),
            Just((0, Some(2)))
        ];
        let leaf = (0_usize..3, occurs.clone()).prop_map(|(id, (min, max))| Particle {
            term: ParticleTerm::Element(id),
            min,
            max,
        });
        leaf.prop_recursive(3, 12, 3, move |inner| {
            (
                prop::collection::vec(inner, 0..4),
                any::<bool>(),
                occurs.clone(),
            )
                .prop_map(|(items, sequence, (min, max))| Particle {
                    term: if sequence {
                        ParticleTerm::Sequence(items)
                    } else {
                        ParticleTerm::Choice(items)
                    },
                    min,
                    max,
                })
        })
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(3000))]

        #[test]
        fn automaton_agrees_with_backtracking(particle in particle_strategy(), word in proptest::collection::vec(0_usize..3, 0..7)) {
            let elements = decls(&["a", "b", "c"]);
            if let Ok(automaton) = Automaton::compile(&particle, &elements) {
                let names = ["a", "b", "c"];
                let letters: Vec<&str> = word.iter().map(|&i| names[i]).collect();
                let expected = ends(&particle, &word, 0).contains(&word.len());
                proptest::prop_assert_eq!(run(&automaton, &elements, &letters), expected);
            }
        }
    }

    #[test]
    fn random_models_are_mostly_usable() {
        use proptest::strategy::{Strategy, ValueTree};
        use proptest::test_runner::TestRunner;
        let elements = decls(&["a", "b", "c"]);
        let mut runner = TestRunner::deterministic();
        let (mut compiled, mut accepted, mut rejected) = (0, 0, 0);
        for _ in 0..2000 {
            let particle = particle_strategy().new_tree(&mut runner).unwrap().current();
            let Ok(automaton) = Automaton::compile(&particle, &elements) else {
                continue;
            };
            compiled += 1;
            for word in [&["a"][..], &["a", "b"], &["b", "c", "a"], &[]] {
                if run(&automaton, &elements, word) {
                    accepted += 1;
                } else {
                    rejected += 1;
                }
            }
        }
        assert!(
            compiled > 500,
            "{compiled} deterministic models out of 2000"
        );
        assert!(
            accepted > 200 && rejected > 200,
            "{accepted} accepted, {rejected} rejected"
        );
    }

    #[test]
    fn non_deterministic_models_are_refused() {
        let elements = decls(&["a", "b"]);
        // (a?, a)
        let model = Particle {
            term: ParticleTerm::Sequence(vec![element(0, 0, Some(1)), element(0, 1, Some(1))]),
            min: 1,
            max: Some(1),
        };
        assert!(Automaton::compile(&model, &elements).is_err());
        // (##any | a)
        let model = Particle {
            term: ParticleTerm::Choice(vec![
                Particle {
                    term: ParticleTerm::Wildcard(Wildcard {
                        namespaces: NamespaceConstraint::Any,
                        process: ProcessContents::Lax,
                    }),
                    min: 1,
                    max: Some(1),
                },
                element(0, 1, Some(1)),
            ]),
            min: 1,
            max: Some(1),
        };
        assert!(Automaton::compile(&model, &elements).is_err());
        // (##other{urn:t} | a) is deterministic: a is in urn:t.
        let model = Particle {
            term: ParticleTerm::Choice(vec![
                Particle {
                    term: ParticleTerm::Wildcard(Wildcard {
                        namespaces: NamespaceConstraint::Other(Some("urn:t".to_owned())),
                        process: ProcessContents::Lax,
                    }),
                    min: 1,
                    max: Some(1),
                },
                element(0, 1, Some(1)),
            ]),
            min: 1,
            max: Some(1),
        };
        assert!(Automaton::compile(&model, &elements).is_ok());
    }
}
