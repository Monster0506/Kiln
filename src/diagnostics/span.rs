/// A half-open byte range [start, end) into the source string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_len() {
        let s = Span::new(4, 9);
        assert_eq!(s.len(), 5);
    }

    #[test]
    fn span_empty() {
        assert!(Span::new(3, 3).is_empty());
        assert!(!Span::new(3, 4).is_empty());
    }
}
