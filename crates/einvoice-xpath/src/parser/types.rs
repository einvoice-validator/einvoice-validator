//! Sequence types and single types (\[49\]–\[53\]).

use super::{NameRole, Parser};
use crate::ast::{AtomicKind, AtomicType, ItemType, Occurrence, SequenceType, SingleType};
use crate::error::{ErrorKind, Result};
use crate::lexer::Tok;
use crate::names::XS_NAMESPACE;

impl Parser<'_> {
    /// \[50\] `SequenceType ::= ("empty-sequence" "(" ")") | (ItemType OccurrenceIndicator?)`.
    /// An occurrence indicator directly after the item type always belongs to it
    /// (constraint "occurrence-indicators" of appendix A.1.1).
    pub(crate) fn sequence_type(&mut self) -> Result<SequenceType> {
        let item = self.item_type()?;
        let occurrence = match self.peek()? {
            Tok::Question => Occurrence::ZeroOrOne,
            Tok::Star => Occurrence::ZeroOrMore,
            Tok::Plus => Occurrence::OneOrMore,
            _ => Occurrence::One,
        };
        if occurrence != Occurrence::One {
            self.bump()?;
        }
        Ok(SequenceType { item, occurrence })
    }

    /// \[52\] `ItemType ::= KindTest | ("item" "(" ")") | AtomicType`
    fn item_type(&mut self) -> Result<ItemType> {
        let tok = self.peek()?.clone();
        let Tok::Name { prefix, local } = tok else {
            return Err(self.unexpected("a sequence type"));
        };
        if prefix.is_none() && *self.peek2()? == Tok::LParen {
            return match local.as_str() {
                "item" => Err(self.error_here(ErrorKind::Unsupported, "item type `item()`")),
                "empty-sequence" => {
                    Err(self.error_here(ErrorKind::Unsupported, "sequence type `empty-sequence()`"))
                }
                _ => Ok(ItemType::Kind(self.kind_test()?)),
            };
        }
        Ok(ItemType::Atomic(self.atomic_type()?))
    }

    /// \[49\] `SingleType ::= AtomicType "?"?`
    pub(crate) fn single_type(&mut self) -> Result<SingleType> {
        let atomic = self.atomic_type()?;
        let allows_empty = *self.peek()? == Tok::Question;
        if allows_empty {
            self.bump()?;
        }
        Ok(SingleType {
            atomic,
            allows_empty,
        })
    }

    /// \[53\] `AtomicType ::= QName`, limited to the supported `xs:` types.
    fn atomic_type(&mut self) -> Result<AtomicType> {
        let offset = self.offset()?;
        let name = self.qname(NameRole::Element, "an atomic type name")?;
        if name.namespace.as_deref() != Some(XS_NAMESPACE) {
            return Err(self.error_at(
                ErrorKind::UnknownType,
                offset,
                format!("`{name}` is not an atomic type"),
            ));
        }
        let kind = AtomicKind::from_local_name(&name.local).ok_or_else(|| {
            self.error_at(
                ErrorKind::Unsupported,
                offset,
                format!("atomic type `{name}`"),
            )
        })?;
        Ok(AtomicType { name, kind })
    }
}
