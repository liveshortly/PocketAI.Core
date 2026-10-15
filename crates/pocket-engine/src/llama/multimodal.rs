//! Multimodal (vision/mtmd) support for the llama.cpp engine.
//!
//! Handles mmproj projector file discovery, request inspection for image parts,
//! image loading into MTMD bitmaps, and multimodal prompt tokenization.

use std::path::Path;

use llama_cpp_2::mtmd::{MtmdBitmap, MtmdContext, MtmdInputChunks, MtmdInputText};
use pocket_types::{ChatMessage, ContentPart, EngineKind, ModelFile, PocketError};

/// Finds the multimodal projector (`mmproj`) file from a list of model files.
///
/// Heuristic: Since `ModelFile` has no `role` field in the catalog schema, we locate
/// the projector by checking if the file's `path` contains `"mmproj"` (case-insensitive).
pub fn find_mmproj_file(files: &[ModelFile]) -> Option<&ModelFile> {
    files
        .iter()
        .find(|f| f.path.to_lowercase().contains("mmproj"))
}

/// Returns true if any message in `messages` contains a `ContentPart::ImageRef`.
pub fn has_image_parts(messages: &[ChatMessage]) -> bool {
    messages.iter().any(|m| {
        m.parts
            .iter()
            .any(|p| matches!(p, ContentPart::ImageRef { .. }))
    })
}

/// Inspects request messages to extract all image file paths from `ImageRef` parts in order.
pub fn extract_image_paths(messages: &[ChatMessage]) -> Vec<String> {
    let mut paths = Vec::new();
    for msg in messages {
        for part in &msg.parts {
            if let ContentPart::ImageRef { path } = part {
                paths.push(path.clone());
            }
        }
    }
    paths
}

/// Loads images from disk into `MtmdBitmap`s using the given `MtmdContext`.
///
/// Returns an error mapped to `PocketError::EngineFailed` if any file is missing or unreadable.
pub fn load_image_bitmaps(
    mtmd: &MtmdContext,
    image_paths: &[String],
) -> Result<Vec<MtmdBitmap>, PocketError> {
    let mut bitmaps = Vec::with_capacity(image_paths.len());
    for path in image_paths {
        let p = Path::new(path);
        if !p.exists() {
            return Err(PocketError::EngineFailed {
                engine: EngineKind::LlamaCpp,
                message: format!("image file does not exist at {}", p.display()),
            });
        }
        let bitmap =
            MtmdBitmap::from_file(mtmd, path, false).map_err(|e| PocketError::EngineFailed {
                engine: EngineKind::LlamaCpp,
                message: format!("failed to load image from {path}: {e}"),
            })?;
        bitmaps.push(bitmap);
    }
    Ok(bitmaps)
}

/// Tokenizes a prompt string alongside loaded bitmaps into `MtmdInputChunks`.
///
/// Ensures each bitmap has a corresponding media marker (`<__media__>`) in the prompt text,
/// prepending any missing markers before the prompt text if necessary.
pub fn tokenize_multimodal_prompt(
    mtmd: &MtmdContext,
    prompt: &str,
    bitmaps: &[MtmdBitmap],
) -> Result<MtmdInputChunks, PocketError> {
    let marker = llama_cpp_2::mtmd::mtmd_default_marker();
    let marker_count = prompt.matches(marker).count();
    let final_prompt = if marker_count < bitmaps.len() {
        let mut prefix = String::new();
        for _ in 0..(bitmaps.len() - marker_count) {
            prefix.push_str(marker);
            prefix.push('\n');
        }
        format!("{prefix}{prompt}")
    } else {
        prompt.to_string()
    };

    let input_text = MtmdInputText {
        text: final_prompt,
        add_special: true,
        parse_special: true,
    };
    let bitmap_refs: Vec<&MtmdBitmap> = bitmaps.iter().collect();
    mtmd.tokenize(input_text, &bitmap_refs)
        .map_err(|e| PocketError::EngineFailed {
            engine: EngineKind::LlamaCpp,
            message: format!("failed to tokenize multimodal prompt: {e}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_mmproj_heuristic_picks_matching_file() {
        let files = vec![
            ModelFile {
                path: "model-q4_k_m.gguf".into(),
                size: 1000,
                sha256: "".into(),
                url: "".into(),
            },
            ModelFile {
                path: "mmproj-model-f16.gguf".into(),
                size: 200,
                sha256: "".into(),
                url: "".into(),
            },
        ];

        let found = find_mmproj_file(&files);
        assert!(found.is_some());
        assert_eq!(found.unwrap().path, "mmproj-model-f16.gguf");
    }

    #[test]
    fn find_mmproj_heuristic_case_insensitive() {
        let files = vec![
            ModelFile {
                path: "model.gguf".into(),
                size: 1000,
                sha256: "".into(),
                url: "".into(),
            },
            ModelFile {
                path: "subfolder/MMPROJ_v1.0.gguf".into(),
                size: 200,
                sha256: "".into(),
                url: "".into(),
            },
        ];

        let found = find_mmproj_file(&files);
        assert!(found.is_some());
        assert_eq!(found.unwrap().path, "subfolder/MMPROJ_v1.0.gguf");
    }

    #[test]
    fn find_mmproj_returns_none_when_absent() {
        let files = vec![
            ModelFile {
                path: "model.gguf".into(),
                size: 1000,
                sha256: "".into(),
                url: "".into(),
            },
            ModelFile {
                path: "tokenizer.json".into(),
                size: 10,
                sha256: "".into(),
                url: "".into(),
            },
        ];

        assert!(find_mmproj_file(&files).is_none());
        assert!(find_mmproj_file(&[]).is_none());
    }

    #[test]
    fn has_image_parts_and_extract_paths_detection() {
        // Empty messages
        assert!(!has_image_parts(&[]));
        assert!(extract_image_paths(&[]).is_empty());

        // Text-only messages
        let text_messages = vec![
            ChatMessage {
                role: "user".into(),
                parts: vec![ContentPart::Text {
                    text: "Hello".into(),
                }],
            },
            ChatMessage {
                role: "assistant".into(),
                parts: vec![ContentPart::Text { text: "Hi".into() }],
            },
        ];
        assert!(!has_image_parts(&text_messages));
        assert!(extract_image_paths(&text_messages).is_empty());

        // Single image message
        let single_image_messages = vec![ChatMessage {
            role: "user".into(),
            parts: vec![
                ContentPart::ImageRef {
                    path: "/tmp/img1.png".into(),
                },
                ContentPart::Text {
                    text: "Describe this".into(),
                },
            ],
        }];
        assert!(has_image_parts(&single_image_messages));
        assert_eq!(
            extract_image_paths(&single_image_messages),
            vec!["/tmp/img1.png".to_string()]
        );

        // Multi-image, multi-turn messages
        let multi_turn_messages = vec![
            ChatMessage {
                role: "user".into(),
                parts: vec![
                    ContentPart::ImageRef {
                        path: "/tmp/a.jpg".into(),
                    },
                    ContentPart::Text {
                        text: "First".into(),
                    },
                ],
            },
            ChatMessage {
                role: "assistant".into(),
                parts: vec![ContentPart::Text {
                    text: "Got it".into(),
                }],
            },
            ChatMessage {
                role: "user".into(),
                parts: vec![
                    ContentPart::Text {
                        text: "Now look at".into(),
                    },
                    ContentPart::ImageRef {
                        path: "/tmp/b.png".into(),
                    },
                ],
            },
        ];
        assert!(has_image_parts(&multi_turn_messages));
        assert_eq!(
            extract_image_paths(&multi_turn_messages),
            vec!["/tmp/a.jpg".to_string(), "/tmp/b.png".to_string()]
        );
    }
}
