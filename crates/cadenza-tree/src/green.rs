//! Green tree: the immutable, interned syntax tree structure.
//!
//! The green tree is the core data structure. It's designed to be:
//! - Immutable: once created, never modified
//! - Interned: identical subtrees share the same memory
//! - Efficient: uses arena allocation and avoids unnecessary copies
//!
//! Implementation follows Rowan 0.16 patterns including:
//! - hashbrown with FxHasher for fast lookups
//! - Raw entry API for manual hash computation
//! - Hash propagation from children to parents
//! - Pointer equality for deduplication

use crate::{SyntaxKind, SyntaxText};
use hashbrown::hash_map::RawEntryMut;
use rustc_hash::FxHasher;
use std::{
    fmt,
    hash::{BuildHasherDefault, Hash, Hasher},
    sync::{Arc, Mutex, OnceLock},
};

type HashMap<K, V> = hashbrown::HashMap<K, V, BuildHasherDefault<FxHasher>>;

#[derive(Debug)]
struct NoHash<T>(T);

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
/// 
/// Implementation matches Rowan 0.16: uses a flat children vector and a separate
/// parents stack to track node boundaries. Children are stored with their hashes
/// for efficient parent hash computation.
pub struct GreenNodeBuilder {
    /// Stack of parent nodes, each storing (kind, first_child_index)
    parents: Vec<(SyntaxKind, usize)>,
    /// Flat list of all children elements with their hashes
    children: Vec<(u64, GreenElement)>,
    cache: &'static Cache,
}

/// A checkpoint in the builder that allows starting nodes at previous positions.
///
/// This is useful for implementing left-associative parsing where you need to
/// wrap previously parsed content in a new node.
#[derive(Debug, Clone, Copy)]
pub struct Checkpoint {
    /// The position in the children vector when the checkpoint was taken.
    children_index: usize,
}

impl GreenNodeBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            parents: Vec::new(),
            children: Vec::new(),
            cache: cache(),
        }
    }

    /// Create a checkpoint at the current position.
    ///
    /// This allows you to start a node at this position later using `start_node_at`.
    /// The checkpoint records the current position in the children vector.
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            children_index: self.children.len(),
        }
    }

    /// Start a new node with the given kind.
    pub fn start_node(&mut self, kind: SyntaxKind) {
        let first_child = self.children.len();
        self.parents.push((kind, first_child));
    }

    /// Start a new node at a previous checkpoint.
    ///
    /// This wraps all children added since the checkpoint into the new node.
    /// This is useful for implementing left-associative parsing where you
    /// wrap previously parsed content.
    pub fn start_node_at(&mut self, checkpoint: Checkpoint, kind: SyntaxKind) {
        let checkpoint_pos = checkpoint.children_index;
        
        // Validate checkpoint is still valid
        assert!(
            checkpoint_pos <= self.children.len(),
            "checkpoint no longer valid, was finish_node called early?"
        );
        
        // Validate checkpoint is not before current parent's first child
        if let Some(&(_, first_child)) = self.parents.last() {
            assert!(
                checkpoint_pos >= first_child,
                "checkpoint no longer valid, was an unmatched start_node_at called?"
            );
        }
        
        // Push new parent starting at the checkpoint position
        self.parents.push((kind, checkpoint_pos));
    }

    /// Add a token to the current node, interning the text.
    ///
    /// This is more efficient than the generic `token` when you have a string slice,
    /// as it avoids allocation if the token is already cached.
    pub fn token(&mut self, kind: SyntaxKind, text: &str) {
        let (hash, token) = self.cache.token(kind, text);
        self.children.push((hash, GreenElement::Token(token)));
    }

    /// Finish the current node and return to the parent.
    pub fn finish_node(&mut self) {
        let (kind, first_child) = self.parents.pop().expect("no node to finish");
        
        // Create the node with hash
        let (hash, node) = self.cache.node(kind, &mut self.children, first_child);
        
        // Add the finished node back to children
        self.children.push((hash, GreenElement::Node(node)));
    }

    /// Finish building and return the root node.
    ///
    /// This consumes the builder and returns the root node.
    /// The builder must have exactly one child (the root) when this is called.
    pub fn finish(mut self) -> GreenNode {
        assert!(self.parents.is_empty(), "unfinished nodes remain");
        assert_eq!(self.children.len(), 1, "expected exactly one root node");
        
        match self.children.pop().unwrap().1 {
            GreenElement::Node(node) => node,
            GreenElement::Token(_) => panic!("root must be a node, not a token"),
        }
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
/// Implementation follows Rowan's optimizations:
/// - Uses hashbrown's raw entry API for manual hash control
/// - Propagates hashes from children to parents
/// - Skips caching for large nodes (>3 children) 
/// - Uses pointer equality for deduplication
struct Cache {
    nodes: Mutex<HashMap<NoHash<GreenNode>, ()>>,
    tokens: Mutex<HashMap<NoHash<GreenToken>, ()>>,
}

/// Compute hash for a token
fn token_hash(kind: SyntaxKind, text: &str) -> u64 {
    let mut h = FxHasher::default();
    kind.hash(&mut h);
    text.hash(&mut h);
    h.finish()
}

/// Get element ID for pointer equality comparison
fn element_id(elem: &GreenElement) -> *const () {
    match elem {
        GreenElement::Node(node) => Arc::as_ptr(&node.inner) as *const (),
        GreenElement::Token(token) => Arc::as_ptr(&token.inner) as *const (),
    }
}

impl Cache {
    fn new() -> Self {
        Self {
            nodes: Mutex::new(HashMap::default()),
            tokens: Mutex::new(HashMap::default()),
        }
    }

    fn token(&self, kind: SyntaxKind, text: &str) -> (u64, GreenToken) {
        let hash = token_hash(kind, text);
        
        let mut cache = self.tokens.lock().unwrap();
        let entry = cache.raw_entry_mut().from_hash(hash, |token| {
            token.0.kind() == kind && token.0.text().as_str() == text
        });
        
        let token = match entry {
            RawEntryMut::Occupied(entry) => entry.key().0.clone(),
            RawEntryMut::Vacant(entry) => {
                // Intern the text
                let interned = crate::interner::InternedString::new(text);
                let token = GreenToken {
                    inner: Arc::new(GreenTokenData {
                        kind,
                        text: SyntaxText::new(interned),
                    }),
                };
                entry.insert_with_hasher(hash, NoHash(token.clone()), (), |t| {
                    token_hash(t.0.kind(), t.0.text().as_str())
                });
                token
            }
        };
        
        (hash, token)
    }

    fn node(
        &self,
        kind: SyntaxKind,
        children: &mut Vec<(u64, GreenElement)>,
        first_child: usize,
    ) -> (u64, GreenNode) {
        let build_node = |children: &mut Vec<(u64, GreenElement)>| {
            let node_children: Vec<GreenElement> = 
                children.drain(first_child..).map(|(_, elem)| elem).collect();
            let width = node_children.iter().map(|c| c.text_len()).sum();
            GreenNode {
                inner: Arc::new(GreenNodeData {
                    kind,
                    children: node_children.into(),
                    width,
                }),
            }
        };

        let children_ref = &children[first_child..];
        
        // Skip caching for large nodes (>3 children) - Rowan optimization
        if children_ref.len() > 3 {
            let node = build_node(children);
            return (0, node);
        }

        // Compute hash from children hashes
        let hash = {
            let mut h = FxHasher::default();
            kind.hash(&mut h);
            for &(child_hash, _) in children_ref {
                if child_hash == 0 {
                    // Child wasn't cached, so don't cache this node either
                    let node = build_node(children);
                    return (0, node);
                }
                child_hash.hash(&mut h);
            }
            h.finish()
        };

        // Use raw entry API with pointer equality for deduplication
        let mut cache = self.nodes.lock().unwrap();
        let entry = cache.raw_entry_mut().from_hash(hash, |node| {
            node.0.kind() == kind 
                && node.0.children().len() == children_ref.len()
                && node.0.children()
                    .iter()
                    .map(element_id)
                    .eq(children_ref.iter().map(|(_, elem)| element_id(elem)))
        });

        let node = match entry {
            RawEntryMut::Occupied(entry) => {
                // Reuse existing node, discard children
                drop(children.drain(first_child..));
                entry.key().0.clone()
            }
            RawEntryMut::Vacant(entry) => {
                let node = build_node(children);
                entry.insert_with_hasher(hash, NoHash(node.clone()), (), |n| {
                    // Recompute hash for insertion
                    let mut h = FxHasher::default();
                    n.0.kind().hash(&mut h);
                    for child in n.0.children() {
                        // This is inefficient but only happens on insertion
                        let child_hash = match child {
                            GreenElement::Node(n) => {
                                let mut h2 = FxHasher::default();
                                n.kind().hash(&mut h2);
                                h2.finish()
                            }
                            GreenElement::Token(t) => token_hash(t.kind(), t.text().as_str()),
                        };
                        child_hash.hash(&mut h);
                    }
                    h.finish()
                });
                node
            }
        };

        (hash, node)
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
