use std::marker::PhantomData;
use std::ops::{Index, IndexMut};

mod sealed {
    pub trait Sealed {}
}

/// An index type that can address one specific kind of arena.
///
/// The trait is sealed so downstream crates use the compiler's ID types
/// rather than creating IDs whose invariants the HIR cannot control.
pub trait ArenaId: sealed::Sealed + Copy + Eq {
    fn from_index(index: usize) -> Self;
    fn index(self) -> usize;
}

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u32);

        impl sealed::Sealed for $name {}

        impl ArenaId for $name {
            fn from_index(index: usize) -> Self {
                let index = u32::try_from(index).expect("arena contains more than u32::MAX items");
                Self(index)
            }

            fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

define_id!(
    /// A top-level declaration such as a state, function, or view.
    DefId
);
define_id!(
    /// A binding introduced inside a body.
    LocalId
);
define_id!(
    /// An expression node.
    ExprId
);
define_id!(
    /// A statement block.
    BlockId
);

/// A dense, append-only store addressed by one specific ID type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arena<I, T> {
    items: Vec<T>,
    id: PhantomData<fn() -> I>,
}

impl<I, T> Arena<I, T>
where
    I: ArenaId,
{
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            id: PhantomData,
        }
    }

    pub fn alloc(&mut self, value: T) -> I {
        let id = I::from_index(self.items.len());
        self.items.push(value);
        id
    }

    pub fn get(&self, id: I) -> &T {
        &self.items[id.index()]
    }

    pub fn get_mut(&mut self, id: I) -> &mut T {
        &mut self.items[id.index()]
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (I, &T)> + DoubleEndedIterator {
        self.items
            .iter()
            .enumerate()
            .map(|(index, value)| (I::from_index(index), value))
    }
}

impl<I, T> Default for Arena<I, T>
where
    I: ArenaId,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<I, T> Index<I> for Arena<I, T>
where
    I: ArenaId,
{
    type Output = T;

    fn index(&self, id: I) -> &Self::Output {
        self.get(id)
    }
}

impl<I, T> IndexMut<I> for Arena<I, T>
where
    I: ArenaId,
{
    fn index_mut(&mut self, id: I) -> &mut Self::Output {
        self.get_mut(id)
    }
}
