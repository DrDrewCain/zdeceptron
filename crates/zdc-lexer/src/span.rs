/// A byte range into the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Self {
        debug_assert!(start <= end, "span start must not exceed end");
        Span { start, end }
    }

    /// A span covering from the start of `self` to the end of `other`.
    pub fn to(self, other: Span) -> Span {
        Span::new(self.start, other.end)
    }

    pub fn len(self) -> u32 {
        self.end - self.start
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

impl From<Span> for std::ops::Range<usize> {
    fn from(s: Span) -> Self {
        s.start as usize..s.end as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_to_covers_both_ends() {
        let a = Span::new(3, 7);
        let b = Span::new(11, 20);
        assert_eq!(a.to(b), Span::new(3, 20));
    }

    #[test]
    fn span_len_is_end_minus_start() {
        assert_eq!(Span::new(4, 10).len(), 6);
    }
}
