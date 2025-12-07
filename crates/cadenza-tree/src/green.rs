//! Green tree: the immutable, interned syntax tree structure.
//!
//! The green tree is the core data structure. It's designed to be:
//! - Immutable: once created, never modified
//! - Interned: identical subtrees share the same memory
//! - Efficient: uses arena allocation and avoids unnecessary copies
//!
//! Nodes and tokens are stored in an arena and referenced by index.

use crate::{SyntaxKind, SyntaxText};
use rustc_hash::FxHashMap;
use std::{
    fmt,
    sync::{Arc, Mutex, OnceLock},
};

/// A node in the green tree.
///
/// Green nodes are immutable and interned, enabling efficient structural sharing.
/// A node consists of a kind and a list of children (which can be nodes or tokens).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct GreenNode {
    inner: Arc<GreenNodeData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GreenNodeData {
    kind: SyntaxKind,
    children: Box<[GreenElement]>,
    width: usize, // cached text length
}

impl GreenNode {
    /// Get the syntax kind of this node.
    #[inline]
    pub fn kind(&self) -> SyntaxKind {
        self.inner.kind
    }

    /// Get the children of this node.
    #[inline]
    pub fn children(&self) -> &[GreenElement] {
        &self.inner.children
    }

    /// Get the text width (byte length) of this node.
    #[inline]
    pub fn text_len(&self) -> usize {
        self.inner.width
    }

    /// Check if this node is empty (no children).
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.inner.children.is_empty()
    }
}

impl fmt::Debug for GreenNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GreenNode")
            .field("kind", &self.inner.kind)
            .field("width", &self.inner.width)
            .field("children", &self.inner.children)
            .finish()
    }
}

/// A token in the green tree.
///
/// Tokens are leaf nodes that contain the actual text.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct GreenToken {
    inner: Arc<GreenTokenData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GreenTokenData {
    kind: SyntaxKind,
    text: SyntaxText,
}

impl GreenToken {
    /// Create a new green token.
    pub fn new(kind: SyntaxKind, text: impl Into<SyntaxText>) -> Self {
        Self {
            inner: Arc::new(GreenTokenData {
                kind,
                text: text.into(),
            }),
        }
    }

    /// Get the syntax kind of this token.
    #[inline]
    pub fn kind(&self) -> SyntaxKind {
        self.inner.kind
    }

    /// Get the text of this token.
    #[inline]
    pub fn text(&self) -> &SyntaxText {
        &self.inner.text
    }

    /// Get the text length of this token.
    #[inline]
    pub fn text_len(&self) -> usize {
        self.inner.text.len()
    }
}

impl fmt::Debug for GreenToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GreenToken")
            .field("kind", &self.inner.kind)
            .field("text", &self.inner.text)
            .finish()
    }
}

/// A green tree element: either a node or a token.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GreenElement {
    Node(GreenNode),
    Token(GreenToken),
}

impl GreenElement {
    /// Get the syntax kind of this element.
    pub fn kind(&self) -> SyntaxKind {
        match self {
            GreenElement::Node(node) => node.kind(),
            GreenElement::Token(token) => token.kind(),
        }
    }

    /// Get the text length of this element.
    pub fn text_len(&self) -> usize {
        match self {
            GreenElement::Node(node) => node.text_len(),
            GreenElement::Token(token) => token.text_len(),
        }
    }

    /// Convert to a node, if this is a node.
    pub fn as_node(&self) -> Option<&GreenNode> {
        match self {
            GreenElement::Node(node) => Some(node),
            GreenElement::Token(_) => None,
        }
    }

    /// Convert to a token, if this is a token.
    pub fn as_token(&self) -> Option<&GreenToken> {
        match self {
            GreenElement::Node(_) => None,
            GreenElement::Token(token) => Some(token),
        }
    }
}

impl From<GreenNode> for GreenElement {
    fn from(node: GreenNode) -> Self {
        GreenElement::Node(node)
    }
}

impl From<GreenToken> for GreenElement {
    fn from(token: GreenToken) -> Self {
        GreenElement::Token(token)
    }
}

/// Builder for constructing green trees.
///
/// This provides an API similar to Rowan's GreenNodeBuilder for easy migration.
pub struct GreenNodeBuilder {
    stack: Vec<(SyntaxKind, Vec<GreenElement>)>,
    cache: &'static Cache,
    root: Option<GreenNode>,
}

/// A checkpoint in the builder that allows starting nodes at previous positions.
///
/// This is useful for implementing left-associative parsing where you need to
/// wrap previously parsed content in a new node.
#[derive(Debug, Clone, Copy)]
pub struct Checkpoint {
    /// The number of children in the parent node when the checkpoint was taken.
    children_count: usize,
}

impl GreenNodeBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            cache: cache(),
            root: None,
        }
    }

    /// Create a checkpoint at the current position.
    ///
    /// This allows you to start a node at this position later using `start_node_at`.
    /// The checkpoint records the current number of children in the current node.
    pub fn checkpoint(&self) -> Checkpoint {
        let children_count = self
            .stack
            .last()
            .map(|(_, children)| children.len())
            .unwrap_or(0);
        Checkpoint { children_count }
    }

    /// Start a new node with the given kind.
    pub fn start_node(&mut self, kind: SyntaxKind) {
        self.stack.push((kind, Vec::new()));
    }

    /// Start a new node at a previous checkpoint.
    ///
    /// This moves all children added since the checkpoint into the new node.
    /// This is useful for implementing left-associative parsing where you
    /// wrap previously parsed content.
    pub fn start_node_at(&mut self, checkpoint: Checkpoint, kind: SyntaxKind) {
        let (_, current_children) = self.stack.last_mut().expect("no current node");
        let children_to_move = current_children.split_off(checkpoint.children_count);

        // Start a new node and move the children into it
        self.stack.push((kind, children_to_move));
    }

    /// Add a token to the current node, interning the text.
    ///
    /// This is more efficient than the generic `token` when you have a string slice,
    /// as it avoids allocation if the token is already cached.
    pub fn token(&mut self, kind: SyntaxKind, text: &str) {
        let token = self.cache.token(kind, text);
        self.add_element(GreenElement::Token(token));
    }

    /// Finish the current node and return to the parent.
    pub fn finish_node(&mut self) {
        let (kind, children) = self.stack.pop().expect("no node to finish");
        let node = self.cache.node(kind, children);

        if self.stack.is_empty() {
            // This is the root node
            self.root = Some(node);
        } else {
            // Add to parent
            self.add_element(GreenElement::Node(node));
        }
    }

    fn add_element(&mut self, element: GreenElement) {
        if let Some((_, children)) = self.stack.last_mut() {
            children.push(element);
        } else {
            panic!("tried to add element with no current node");
        }
    }

    /// Finish building and return the root node.
    ///
    /// # Panics
    /// Panics if there are unfinished nodes or no root was created.
    pub fn finish(self) -> GreenNode {
        assert!(
            self.stack.is_empty(),
            "unfinished nodes remain in builder"
        );
        self.root.expect("no root node was created")
    }
}

impl Default for GreenNodeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Cache for interning green nodes and tokens.
///
/// This ensures that identical subtrees and tokens share the same memory.
struct Cache {
    nodes: Mutex<FxHashMap<(SyntaxKind, Box<[GreenElement]>), GreenNode>>,
    tokens: Mutex<FxHashMap<(SyntaxKind, String), GreenToken>>,
}

impl Cache {
    fn new() -> Self {
        Self {
            nodes: Mutex::new(FxHashMap::default()),
            tokens: Mutex::new(FxHashMap::default()),
        }
    }

    fn token(&self, kind: SyntaxKind, text: &str) -> GreenToken {
        let mut cache = self.tokens.lock().unwrap();
        
        // Use entry API to avoid double lookup
        use std::collections::hash_map::Entry;
        match cache.entry((kind, text.to_string())) {
            Entry::Occupied(e) => e.get().clone(),
            Entry::Vacant(e) => {
                // Create interned token
                let text_arc: Arc<str> = Arc::from(text);
                let token = GreenToken {
                    inner: Arc::new(GreenTokenData {
                        kind,
                        text: SyntaxText::interned(text_arc),
                    }),
                };
                e.insert(token.clone());
                token
            }
        }
    }

    fn node(&self, kind: SyntaxKind, children: Vec<GreenElement>) -> GreenNode {
        let children: Box<[GreenElement]> = children.into();

        let mut cache = self.nodes.lock().unwrap();
        
        // Check if already cached
        if let Some(node) = cache.get(&(kind, children.clone())) {
            return node.clone();
        }

        // Not found, create new node
        let width = children.iter().map(|c| c.text_len()).sum();
        let node = GreenNode {
            inner: Arc::new(GreenNodeData {
                kind,
                children: children.clone(),
                width,
            }),
        };

        // Insert into cache (still holding the lock)
        cache.insert((kind, children), node.clone());
        node
    }
}

/// Get the global cache.
fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Cache::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_green_token() {
        let token = GreenToken::new(SyntaxKind::new(1), "hello");
        assert_eq!(token.kind(), SyntaxKind::new(1));
        assert_eq!(token.text().as_str(), "hello");
        assert_eq!(token.text_len(), 5);
    }

    #[test]
    fn test_green_node_builder() {
        let mut builder = GreenNodeBuilder::new();

        builder.start_node(SyntaxKind::new(1));
        builder.token(SyntaxKind::new(2), "hello");
        builder.token(SyntaxKind::new(3), " ");
        builder.token(SyntaxKind::new(2), "world");
        builder.finish_node();

        let root = builder.finish();
        assert_eq!(root.kind(), SyntaxKind::new(1));
        assert_eq!(root.children().len(), 3);
        assert_eq!(root.text_len(), 11);
    }

    #[test]
    fn test_node_interning() {
        let mut builder1 = GreenNodeBuilder::new();
        builder1.start_node(SyntaxKind::new(1));
        builder1.token(SyntaxKind::new(2), "test");
        builder1.finish_node();
        let node1 = builder1.finish();

        let mut builder2 = GreenNodeBuilder::new();
        builder2.start_node(SyntaxKind::new(1));
        builder2.token(SyntaxKind::new(2), "test");
        builder2.finish_node();
        let node2 = builder2.finish();

        // Same structure should be interned (same Arc)
        assert_eq!(node1, node2);
        assert!(Arc::ptr_eq(&node1.inner, &node2.inner));
    }
}
