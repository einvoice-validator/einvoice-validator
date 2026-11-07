//! Items: nodes and atomic values.

use std::borrow::Cow;

use einvoice_xdm::AtomicValue;

use crate::tree::Node;

/// An item of a sequence.
#[derive(Debug, Clone)]
pub enum Item {
    /// A node.
    Node(Node),
    /// An atomic value.
    Atomic(AtomicValue),
}

impl Item {
    /// The atomized value: the typed value of a node, the value itself otherwise.
    #[must_use]
    pub fn atomize(&self) -> AtomicValue {
        match self {
            Self::Node(node) => node.typed_value(),
            Self::Atomic(value) => value.clone(),
        }
    }

    /// The string value (`fn:string`).
    #[must_use]
    pub fn string_value(&self) -> Cow<'_, str> {
        match self {
            Self::Node(node) => node.string_value(),
            Self::Atomic(value) => match value {
                AtomicValue::String(s) | AtomicValue::UntypedAtomic(s) | AtomicValue::AnyUri(s) => {
                    Cow::Borrowed(s)
                }
                other => Cow::Owned(other.to_string()),
            },
        }
    }

    /// The node, if the item is one.
    #[must_use]
    pub const fn as_node(&self) -> Option<&Node> {
        match self {
            Self::Node(node) => Some(node),
            Self::Atomic(_) => None,
        }
    }
}

impl From<Node> for Item {
    fn from(node: Node) -> Self {
        Self::Node(node)
    }
}

impl From<AtomicValue> for Item {
    fn from(value: AtomicValue) -> Self {
        Self::Atomic(value)
    }
}
