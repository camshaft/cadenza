//! Red tree: typed wrappers around the green tree.
//!
//! The red tree provides convenient, typed access to the green tree.
//! It calculates absolute positions and provides iteration over children.

use crate::{green::*, Language, SyntaxText};
use std::{fmt, hash::Hash, marker::PhantomData, sync::Arc};

/// A node in the red tree.
///
/// This is a typed wrapper around a GreenNode that tracks its position
/// in the source text and provides convenient access methods.
pub struct SyntaxNode<L: Language> {
    green: GreenNode,
    parent: Option<Arc<SyntaxNode<L>>>,
    offset: usize,
    index_in_parent: usize,
    _phantom: PhantomData<L>,
}

impl<L: Language> Clone for SyntaxNode<L> {
    fn clone(&self) -> Self {
        Self {
            green: self.green.clone(),
            parent: self.parent.clone(),
            offset: self.offset,
            index_in_parent: self.index_in_parent,
            _phantom: PhantomData,
        }
    }
}

impl<L: Language> SyntaxNode<L> {
    /// Create a new root node from a green node.
    pub fn new_root(green: GreenNode) -> Self {
        Self {
            green,
            parent: None,
            offset: 0,
            index_in_parent: 0,
            _phantom: PhantomData,
        }
    }

    /// Get the syntax kind of this node.
    #[inline]
    pub fn kind(&self) -> L::Kind {
        L::kind_from_raw(self.green.kind())
    }

    /// Get the green node.
    #[inline]
    pub fn green(&self) -> &GreenNode {
        &self.green
    }

    /// Get the parent of this node, if any.
    #[inline]
    pub fn parent(&self) -> Option<&SyntaxNode<L>> {
        self.parent.as_deref()
    }

    /// Get the absolute offset of this node in the source text.
    #[inline]
    pub fn text_range(&self) -> TextRange {
        TextRange::new(self.offset, self.offset + self.green.text_len())
    }

    /// Get the text of this node.
    ///
    /// This is computed by concatenating all descendant tokens.
    pub fn text(&self) -> SyntaxText {
        let mut text = String::new();
        self.collect_text(&mut text);
        SyntaxText::owned(text)
    }

    fn collect_text(&self, buf: &mut String) {
        for child in self.green.children() {
            match child {
                GreenElement::Node(node) => {
                    let child_node = self.child_node(node);
                    child_node.collect_text(buf);
                }
                GreenElement::Token(token) => {
                    buf.push_str(token.text().as_str());
                }
            }
        }
    }

    /// Iterate over direct children of this node.
    pub fn children(&self) -> impl Iterator<Item = SyntaxNode<L>> + '_ {
        self.green
            .children()
            .iter()
            .enumerate()
            .filter_map(move |(i, child)| match child {
                GreenElement::Node(node) => Some(self.child_at_index(node.clone(), i)),
                GreenElement::Token(_) => None,
            })
    }

    /// Iterate over all child elements (nodes and tokens).
    pub fn children_with_tokens(&self) -> impl Iterator<Item = SyntaxElement<L>> + '_ {
        self.green
            .children()
            .iter()
            .enumerate()
            .map(move |(i, child)| self.element_at_index(child.clone(), i))
    }

    fn child_node(&self, green: &GreenNode) -> SyntaxNode<L> {
        // Find this child's index
        for (i, child) in self.green.children().iter().enumerate() {
            if let GreenElement::Node(n) = child {
                if n == green {
                    return self.child_at_index(green.clone(), i);
                }
            }
        }
        panic!("child not found in parent");
    }

    fn child_at_index(&self, green: GreenNode, index: usize) -> SyntaxNode<L> {
        let offset = self.offset + self.child_offset(index);
        SyntaxNode {
            green,
            parent: Some(Arc::new(self.clone())),
            offset,
            index_in_parent: index,
            _phantom: PhantomData,
        }
    }

    fn element_at_index(&self, green: GreenElement, index: usize) -> SyntaxElement<L> {
        let offset = self.offset + self.child_offset(index);
        match green {
            GreenElement::Node(node) => SyntaxElement::Node(SyntaxNode {
                green: node,
                parent: Some(Arc::new(self.clone())),
                offset,
                index_in_parent: index,
                _phantom: PhantomData,
            }),
            GreenElement::Token(token) => SyntaxElement::Token(SyntaxToken {
                green: token,
                parent: Arc::new(self.clone()),
                offset,
                index_in_parent: index,
                _phantom: PhantomData,
            }),
        }
    }

    fn child_offset(&self, index: usize) -> usize {
        self.green.children()[..index]
            .iter()
            .map(|c| c.text_len())
            .sum()
    }
}

impl<L: Language> PartialEq for SyntaxNode<L> {
    fn eq(&self, other: &Self) -> bool {
        self.green == other.green && self.offset == other.offset
    }
}

impl<L: Language> Eq for SyntaxNode<L> {}

impl<L: Language> Hash for SyntaxNode<L> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.green.hash(state);
        self.offset.hash(state);
    }
}

impl<L: Language> fmt::Debug for SyntaxNode<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Format like Rowan: Kind@start..end with children indented
        fmt_node(self, f, 0, true)
    }
}

/// Format a syntax node with Rowan-compatible output.
fn fmt_node<L: Language>(
    node: &SyntaxNode<L>,
    f: &mut fmt::Formatter<'_>,
    indent: usize,
    is_first: bool,
) -> fmt::Result {
    if !is_first {
        write!(f, "{:indent$}", "", indent = indent * 2)?;
    }
    
    write!(f, "{:?}@", node.kind())?;
    write!(f, "{}..{}", node.text_range().start().into(), node.text_range().end().into())?;
    
    // Check if this node has any children
    let has_children = !node.green.children().is_empty();
    
    if has_children {
        writeln!(f)?;
        for child in node.children_with_tokens() {
            match child {
                SyntaxElement::Node(child_node) => {
                    fmt_node(&child_node, f, indent + 1, false)?;
                }
                SyntaxElement::Token(token) => {
                    write!(f, "{:indent$}", "", indent = (indent + 1) * 2)?;
                    write!(f, "{:?}@", token.kind())?;
                    write!(f, "{}..{}", token.text_range().start().into(), token.text_range().end().into())?;
                    write!(f, " {:?}", token.text().as_str())?;
                    writeln!(f)?;
                }
            }
        }
    } else if !is_first {
        // Empty node but not the root - still need newline
        writeln!(f)?;
    }
    
    Ok(())
}

/// A token in the red tree.
pub struct SyntaxToken<L: Language> {
    green: GreenToken,
    parent: Arc<SyntaxNode<L>>,
    offset: usize,
    index_in_parent: usize,
    _phantom: PhantomData<L>,
}

impl<L: Language> Clone for SyntaxToken<L> {
    fn clone(&self) -> Self {
        Self {
            green: self.green.clone(),
            parent: self.parent.clone(),
            offset: self.offset,
            index_in_parent: self.index_in_parent,
            _phantom: PhantomData,
        }
    }
}

impl<L: Language> SyntaxToken<L> {
    /// Get the syntax kind of this token.
    #[inline]
    pub fn kind(&self) -> L::Kind {
        L::kind_from_raw(self.green.kind())
    }

    /// Get the green token.
    #[inline]
    pub fn green(&self) -> &GreenToken {
        &self.green
    }

    /// Get the parent of this token.
    #[inline]
    pub fn parent(&self) -> &SyntaxNode<L> {
        &self.parent
    }

    /// Get the absolute offset of this token in the source text.
    #[inline]
    pub fn text_range(&self) -> TextRange {
        TextRange::new(self.offset, self.offset + self.green.text_len())
    }

    /// Get the text of this token.
    #[inline]
    pub fn text(&self) -> &SyntaxText {
        self.green.text()
    }
}

impl<L: Language> PartialEq for SyntaxToken<L> {
    fn eq(&self, other: &Self) -> bool {
        self.green == other.green && self.offset == other.offset
    }
}

impl<L: Language> Eq for SyntaxToken<L> {}

impl<L: Language> Hash for SyntaxToken<L> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.green.hash(state);
        self.offset.hash(state);
    }
}

impl<L: Language> fmt::Debug for SyntaxToken<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SyntaxToken")
            .field("kind", &self.kind())
            .field("text", &self.text())
            .field("range", &self.text_range())
            .finish()
    }
}

/// Either a node or a token in the red tree.
pub enum SyntaxElement<L: Language> {
    Node(SyntaxNode<L>),
    Token(SyntaxToken<L>),
}

impl<L: Language> Clone for SyntaxElement<L> {
    fn clone(&self) -> Self {
        match self {
            SyntaxElement::Node(node) => SyntaxElement::Node(node.clone()),
            SyntaxElement::Token(token) => SyntaxElement::Token(token.clone()),
        }
    }
}

impl<L: Language> SyntaxElement<L> {
    /// Get the syntax kind of this element.
    pub fn kind(&self) -> L::Kind {
        match self {
            SyntaxElement::Node(node) => node.kind(),
            SyntaxElement::Token(token) => token.kind(),
        }
    }

    /// Get the text range of this element.
    pub fn text_range(&self) -> TextRange {
        match self {
            SyntaxElement::Node(node) => node.text_range(),
            SyntaxElement::Token(token) => token.text_range(),
        }
    }

    /// Convert to a node, if this is a node.
    pub fn as_node(&self) -> Option<&SyntaxNode<L>> {
        match self {
            SyntaxElement::Node(node) => Some(node),
            SyntaxElement::Token(_) => None,
        }
    }

    /// Convert to a token, if this is a token.
    pub fn as_token(&self) -> Option<&SyntaxToken<L>> {
        match self {
            SyntaxElement::Node(_) => None,
            SyntaxElement::Token(token) => Some(token),
        }
    }

    /// Convert to a node, consuming self.
    pub fn into_node(self) -> Option<SyntaxNode<L>> {
        match self {
            SyntaxElement::Node(node) => Some(node),
            SyntaxElement::Token(_) => None,
        }
    }

    /// Convert to a token, consuming self.
    pub fn into_token(self) -> Option<SyntaxToken<L>> {
        match self {
            SyntaxElement::Node(_) => None,
            SyntaxElement::Token(token) => Some(token),
        }
    }
}

impl<L: Language> From<SyntaxNode<L>> for SyntaxElement<L> {
    fn from(node: SyntaxNode<L>) -> Self {
        SyntaxElement::Node(node)
    }
}

impl<L: Language> From<SyntaxToken<L>> for SyntaxElement<L> {
    fn from(token: SyntaxToken<L>) -> Self {
        SyntaxElement::Token(token)
    }
}

impl<L: Language> PartialEq for SyntaxElement<L> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (SyntaxElement::Node(a), SyntaxElement::Node(b)) => a == b,
            (SyntaxElement::Token(a), SyntaxElement::Token(b)) => a == b,
            _ => false,
        }
    }
}

impl<L: Language> Eq for SyntaxElement<L> {}

impl<L: Language> Hash for SyntaxElement<L> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            SyntaxElement::Node(node) => {
                0u8.hash(state);
                node.hash(state);
            }
            SyntaxElement::Token(token) => {
                1u8.hash(state);
                token.hash(state);
            }
        }
    }
}

impl<L: Language> fmt::Debug for SyntaxElement<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyntaxElement::Node(node) => node.fmt(f),
            SyntaxElement::Token(token) => token.fmt(f),
        }
    }
}

/// Alias for SyntaxElement for compatibility with Rowan API.
pub type NodeOrToken<L> = SyntaxElement<L>;

/// A text range in the source.
///
/// This is compatible with Rowan's TextRange.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextRange {
    start: usize,
    end: usize,
}

impl TextRange {
    /// Create a new text range.
    pub fn new(start: usize, end: usize) -> Self {
        assert!(start <= end);
        Self { start, end }
    }

    /// Get the start offset.
    #[inline]
    pub fn start(&self) -> TextSize {
        TextSize(self.start)
    }

    /// Get the end offset.
    #[inline]
    pub fn end(&self) -> TextSize {
        TextSize(self.end)
    }

    /// Get the length of the range.
    #[inline]
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Check if the range is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// A text size (offset) in the source.
///
/// This is compatible with Rowan's TextSize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextSize(usize);

impl TextSize {
    /// Get the raw usize value.
    #[inline]
    pub fn into(self) -> usize {
        self.0
    }
}

impl From<usize> for TextSize {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl From<TextSize> for usize {
    fn from(size: TextSize) -> Self {
        size.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SyntaxKind;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum TestKind {
        Root = 1,
        Ident = 2,
        Whitespace = 3,
    }

    #[derive(Debug, Clone, Copy)]
    struct TestLang;

    impl Language for TestLang {
        type Kind = TestKind;

        fn kind_from_raw(raw: SyntaxKind) -> Self::Kind {
            match raw.into_raw() {
                1 => TestKind::Root,
                2 => TestKind::Ident,
                3 => TestKind::Whitespace,
                _ => panic!("unknown kind"),
            }
        }

        fn kind_to_raw(kind: Self::Kind) -> SyntaxKind {
            SyntaxKind::new(kind as u16)
        }
    }

    #[test]
    fn test_syntax_node_basic() {
        let mut builder = GreenNodeBuilder::new();
        builder.start_node(SyntaxKind::new(1));
        builder.token(SyntaxKind::new(2), "hello");
        builder.token(SyntaxKind::new(3), " ");
        builder.token(SyntaxKind::new(2), "world");
        builder.finish_node();

        let root = SyntaxNode::<TestLang>::new_root(builder.finish());
        assert_eq!(root.kind(), TestKind::Root);
        assert_eq!(root.text().as_str(), "hello world");
        assert_eq!(root.text_range().len(), 11);
    }

    #[test]
    fn test_syntax_node_children() {
        let mut builder = GreenNodeBuilder::new();
        builder.start_node(SyntaxKind::new(1));
        builder.start_node(SyntaxKind::new(1));
        builder.token(SyntaxKind::new(2), "a");
        builder.finish_node();
        builder.start_node(SyntaxKind::new(1));
        builder.token(SyntaxKind::new(2), "b");
        builder.finish_node();
        builder.finish_node();

        let root = SyntaxNode::<TestLang>::new_root(builder.finish());
        let children: Vec<_> = root.children().collect();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].text().as_str(), "a");
        assert_eq!(children[1].text().as_str(), "b");
    }
}
