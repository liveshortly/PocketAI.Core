use rusqlite::params;

use crate::Store;

#[derive(Debug, Clone, PartialEq)]
pub struct ChunkRow {
    pub id: String,
    pub document_id: String,
    pub ord: u32,
    pub text: String,
    pub page: Option<u32>,
    pub embedding: Vec<u8>,
}

pub struct ChunksRepo<'a> {
    store: &'a Store,
}

impl<'a> ChunksRepo<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    pub fn create(&self, row: &ChunkRow) -> rusqlite::Result<()> {
        let conn = self.store.writer();
        conn.execute(
            "INSERT INTO chunks (id, document_id, ord, text, page, embedding) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![row.id, row.document_id, row.ord, row.text, row.page, row.embedding],
        )?;
        conn.execute(
            "INSERT INTO chunks_fts (rowid, text) VALUES (last_insert_rowid(), ?1)",
            params![row.text],
        )?;
        Ok(())
    }

    pub fn list_for_document(&self, document_id: &str) -> rusqlite::Result<Vec<ChunkRow>> {
        let conn = self.store.reader();
        let mut stmt = conn.prepare(
            "SELECT id, document_id, ord, text, page, embedding FROM chunks WHERE document_id = ?1 ORDER BY ord",
        )?;
        let rows = stmt.query_map([document_id], |row| {
            Ok(ChunkRow {
                id: row.get(0)?,
                document_id: row.get(1)?,
                ord: row.get(2)?,
                text: row.get(3)?,
                page: row.get(4)?,
                embedding: row.get(5)?,
            })
        })?;
        rows.collect()
    }

    pub fn delete(&self, id: &str) -> rusqlite::Result<()> {
        self.store
            .writer()
            .execute("DELETE FROM chunks WHERE id = ?1", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repos::documents::{DocumentRow, DocumentsRepo};
    use crate::repos::projects::{ProjectRow, ProjectsRepo};

    #[test]
    fn crud_round_trip() {
        let store = Store::open_in_memory().unwrap();
        ProjectsRepo::new(&store)
            .create(&ProjectRow {
                id: "p1".into(),
                name: "P".into(),
                instructions: None,
                created_at: 1,
            })
            .unwrap();
        DocumentsRepo::new(&store)
            .create(&DocumentRow {
                id: "d1".into(),
                project_id: "p1".into(),
                name: "doc".into(),
                mime: "text/plain".into(),
                bytes: 1,
                path: "/tmp/d".into(),
                text_chars: 10,
                embed_model: None,
                state: "ready".into(),
                created_at: 1,
            })
            .unwrap();
        let repo = ChunksRepo::new(&store);
        repo.create(&ChunkRow {
            id: "c1".into(),
            document_id: "d1".into(),
            ord: 0,
            text: "hello world".into(),
            page: Some(1),
            embedding: vec![0, 1, 2, 3],
        })
        .unwrap();
        assert_eq!(repo.list_for_document("d1").unwrap().len(), 1);
        repo.delete("c1").unwrap();
        assert_eq!(repo.list_for_document("d1").unwrap().len(), 0);
    }
}
