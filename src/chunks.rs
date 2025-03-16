pub struct ChunksChanges<'a, T: PartialEq> {
    v: &'a [T],
    offset: usize,
    chunk_size: usize,
    reference: &'a [T],
}

impl<'a, T: PartialEq> Iterator for ChunksChanges<'a, T> {
    type Item = (usize, &'a [T]);

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset >= self.v.len() {
            return None;
        }

        if let Some((index, _)) = self.v[self.offset..]
            .iter()
            .enumerate()
            .find(|(i, v)| **v != self.reference[*i + self.offset])
        {
            let start = self.offset + index;
            let max_end = (start + self.chunk_size).min(self.v.len());
            let (count, _) = self.v[start..max_end]
                .iter()
                .enumerate()
                .rfind(|(i, v)| **v != self.reference[*i + start])
                .unwrap();

            self.offset = max_end;
            return Some((start, &self.v[start..=start + count]));
        }

        return None;
    }
}

pub trait ChunkChanged<'a, T: PartialEq + 'a> {
    type Iter: Iterator<Item = (usize, &'a [T])>;

    fn chunk_changed(&'a self, chunk_size: usize, reference: &'a [T]) -> Self::Iter;
}

impl<'a, T: PartialEq + 'a> ChunkChanged<'a, T> for [T] {
    type Iter = ChunksChanges<'a, T>;

    fn chunk_changed(&'a self, chunk_size: usize, reference: &'a [T]) -> Self::Iter {
        assert!(chunk_size != 0, "chunk size must be non-zero");
        assert!(
            self.len() <= reference.len(),
            "reference must be at least as long as self"
        );

        ChunksChanges {
            v: self,
            offset: 0,
            chunk_size,
            reference,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_example() {
        let original = vec![1, 3, 4, 5, 6, 6, 8];
        let modified = vec![4, 5, 7];
        let offset = 2;
        let chunk_size = 2;

        let chunks: Vec<_> = modified
            .chunk_changed(chunk_size, &original[offset..])
            .collect();
        assert_eq!(chunks, vec![(2, &[7][..])]);
    }

    #[test]
    fn multiple_chunks_example() {
        let original = vec![1, 3, 4, 5, 6, 6, 8];
        let modified = vec![1, -1, 4, -2, 6, -3, 8];
        let chunk_size = 3;

        let chunks: Vec<_> = modified.chunk_changed(chunk_size, &original).collect();
        assert_eq!(chunks, vec![(1, &[-1, 4, -2][..]), (5, &[-3][..])]);
    }

    #[test]
    fn all_elements_differ() {
        let original = vec![1, 2, 3];
        let modified = vec![4, 5, 6];
        let chunks: Vec<_> = modified.chunk_changed(2, &original).collect();
        assert_eq!(chunks, vec![(0, &[4, 5][..]), (2, &[6][..])]);
    }

    #[test]
    fn no_differences() {
        let original = vec![1, 2, 3];
        let modified = vec![1, 2, 3];
        let chunks: Vec<_> = modified.chunk_changed(2, &original).collect();
        assert!(chunks.is_empty());
    }

    #[test]
    fn exact_chunk_boundaries() {
        let original = vec![0, 0, 0, 0, 0];
        let modified = vec![1, 0, 2, 0, 3];
        let chunks: Vec<_> = modified.chunk_changed(2, &original).collect();
        assert_eq!(chunks, vec![(0, &[1][..]), (2, &[2][..]), (4, &[3][..])]);
    }

    #[test]
    fn large_chunk_size() {
        let original = vec![1, 2, 3];
        let modified = vec![4, 5, 6];
        let chunks: Vec<_> = modified.chunk_changed(5, &original).collect();
        assert_eq!(chunks, vec![(0, &[4, 5, 6][..])]);
    }
}
