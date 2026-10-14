//! ENG-11's chunker: splits each page's text by paragraph/heading
//! boundaries into chunks targeting ~1,800 chars with ~15% overlap between
//! consecutive chunks, keeping the originating page number on every chunk.
//! Never merges text across a page boundary, since the page number is part
//! of each chunk's identity (and of the eventual citation).
pub struct ChunkerConfig {
    pub target_chars: usize,
    pub overlap_fraction: f32,
}

impl Default for ChunkerConfig {
    fn default() -> Self {
        Self { target_chars: 1800, overlap_fraction: 0.15 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageText {
    pub page: u32,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub page: u32,
    pub text: String,
}

pub fn chunk_document(pages: &[PageText], config: &ChunkerConfig) -> Vec<Chunk> {
    pages.iter().flat_map(|page| chunk_page(page, config)).collect()
}

fn chunk_page(page: &PageText, config: &ChunkerConfig) -> Vec<Chunk> {
    let paragraphs: Vec<&str> = page.text.split("\n\n").map(|p| p.trim()).filter(|p| !p.is_empty()).collect();
    if paragraphs.is_empty() {
        return Vec::new();
    }

    let overlap_chars = (config.target_chars as f32 * config.overlap_fraction) as usize;
    let mut chunks = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut current_len = 0usize;

    for paragraph in paragraphs {
        current_len += paragraph.len() + 2;
        current.push(paragraph);

        if current_len >= config.target_chars {
            chunks.push(Chunk { page: page.page, text: current.join("\n\n") });
            // Carry over trailing paragraphs worth ~overlap_chars into the
            // next chunk's start, so consecutive chunks share context.
            let mut carry: Vec<&str> = Vec::new();
            let mut carry_len = 0usize;
            for p in current.iter().rev() {
                if carry_len >= overlap_chars {
                    break;
                }
                carry_len += p.len() + 2;
                carry.insert(0, p);
            }
            current = carry;
            current_len = carry_len;
        }
    }

    if !current.is_empty() {
        chunks.push(Chunk { page: page.page, text: current.join("\n\n") });
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paragraph(len: usize, marker: char) -> String {
        std::iter::repeat_n(marker, len).collect()
    }

    #[test]
    fn a_short_page_becomes_a_single_chunk_with_its_page_number() {
        let pages = vec![PageText { page: 3, text: "short content".into() }];
        let chunks = chunk_document(&pages, &ChunkerConfig::default());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].page, 3);
        assert_eq!(chunks[0].text, "short content");
    }

    #[test]
    fn a_long_page_splits_into_multiple_chunks_near_the_target_size() {
        // 5 paragraphs of 500 chars each = 2500 chars total, well past the
        // 1800-char target - must split into at least 2 chunks.
        let text = (0..5).map(|i| paragraph(500, (b'a' + i) as char)).collect::<Vec<_>>().join("\n\n");
        let pages = vec![PageText { page: 1, text }];
        let config = ChunkerConfig::default();
        let chunks = chunk_document(&pages, &config);

        assert!(chunks.len() >= 2, "expected the long page to split, got {} chunk(s)", chunks.len());
        for chunk in &chunks {
            assert_eq!(chunk.page, 1);
            assert!(chunk.text.len() <= config.target_chars * 2, "chunk grew unreasonably large: {} chars", chunk.text.len());
        }
    }

    #[test]
    fn consecutive_chunks_share_overlapping_content() {
        let text = (0..6).map(|i| paragraph(400, (b'a' + i) as char)).collect::<Vec<_>>().join("\n\n");
        let pages = vec![PageText { page: 1, text }];
        let chunks = chunk_document(&pages, &ChunkerConfig::default());
        assert!(chunks.len() >= 2);

        // The first paragraph of chunk N+1 should also appear at the tail
        // of chunk N (that's the carried-over overlap).
        for pair in chunks.windows(2) {
            let next_first_paragraph = pair[1].text.split("\n\n").next().unwrap();
            assert!(
                pair[0].text.contains(next_first_paragraph),
                "expected chunk overlap: {:?} should contain {:?}",
                pair[0].text,
                next_first_paragraph
            );
        }
    }

    #[test]
    fn chunks_never_cross_a_page_boundary() {
        let pages = vec![
            PageText { page: 1, text: "page one content".into() },
            PageText { page: 2, text: "page two content".into() },
        ];
        let chunks = chunk_document(&pages, &ChunkerConfig::default());
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].page, 1);
        assert_eq!(chunks[1].page, 2);
    }

    #[test]
    fn blank_pages_produce_no_chunks() {
        let pages = vec![PageText { page: 1, text: "   \n\n  ".into() }];
        assert!(chunk_document(&pages, &ChunkerConfig::default()).is_empty());
    }
}
