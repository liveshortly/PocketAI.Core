use rusqlite::params;

use crate::Store;

#[derive(Debug, Clone, PartialEq)]
pub struct DocumentRow {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub mime: String,
    pub bytes: u64,
    pub path: String,
    pub text_chars: u32,
    pub embed_model: Option<String>,
    pub state: String,
    pub created_at: i64,
}

pub struct DocumentsRepo<'a> {
    store: &'a Store,
}

impl<'a> DocumentsRepo<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    pub fn create(&self, row: &DocumentRow) -> rusqlite::Result<()> {
        self.store.writer().execute(
            "INSERT INTO documents (id, project_id, name, mime, bytes, path, text_chars, embed_model, state, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![row.id, row.project_id, row.name, row.mime, row.bytes as i64, row.path,
                row.text_chars, row.embed_model, row.state, row.created_at],
        )?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> rusqlite::Result<Option<DocumentRow>> {
        self.store
            .reader()
            .query_row(
                "SELECT id, project_id, name, mime, bytes, path, text_chars, embed_model, state, created_at
                 FROM documents WHERE id = ?1",
                [id],
                Self::map_row,
            )
            .map(Some)
            .or_else(|e| if e == rusqlite::Error::QueryReturnedNoRows { Ok(None) } else { Err(e) })
    }

    pub fn list_for_project(&self, project_id: &str) -> rusqlite::Result<Vec<DocumentRow>> {
        let conn = self.store.reader();
        let mut stmt = conn.prepare(
            "SELECT id, project_id, name, mime, bytes, path, text_chars, embed_model, state, created_at
             FROM documents WHERE project_id = ?1",
        )?;
        let rows = stmt.query_map([project_id], Self::map_row)?;
        rows.collect()
    }

    pub fn delete(&self, id: &str) -> rusqlite::Result<()> {
        self.store.writer().execute("DELETE FROM documents WHERE id = ?1", [id])?;
        Ok(())
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<DocumentRow> {
        Ok(DocumentRow {
            id: row.get(0)?,
            project_id: row.get(1)?,
            name: row.get(2)?,
            mime: row.get(3)?,
            bytes: row.get::<_, i64>(4)? as u64,
            path: row.get(5)?,
            text_chars: row.get(6)?,
            embed_model: row.get(7)?,
            state: row.get(8)?,
            created_at: row.get(9)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repos::projects::{ProjectRow, ProjectsRepo};

    fn project(id: &str) -> ProjectRow {
        ProjectRow {
            id: id.to_string(),
            name: "P".into(),
            instructions: None,
            created_at: 1,
        }
    }

    fn document(id: &str, project_id: &str) -> DocumentRow {
        DocumentRow {
            id: id.to_string(),
            project_id: project_id.to_string(),
            name: "doc.pdf".into(),
            mime: "application/pdf".into(),
            bytes: 100,
            path: "/tmp/doc.pdf".into(),
            text_chars: 0,
            embed_model: None,
            state: "pending".into(),
            created_at: 1,
        }
    }

    #[test]
    fn crud_round_trip() {
        let store = Store::open_in_memory().unwrap();
        ProjectsRepo::new(&store).create(&project("p1")).unwrap();
        let repo = DocumentsRepo::new(&store);
        repo.create(&document("d1", "p1")).unwrap();
        assert_eq!(repo.get("d1").unwrap().unwrap().name, "doc.pdf");
        assert_eq!(repo.list_for_project("p1").unwrap().len(), 1);
        repo.delete("d1").unwrap();
        assert_eq!(repo.get("d1").unwrap(), None);
    }

    #[test]
    fn cascade_delete_removes_documents_when_project_deleted() {
        let store = Store::open_in_memory().unwrap();
        let projects = ProjectsRepo::new(&store);
        projects.create(&project("p1")).unwrap();
        let documents = DocumentsRepo::new(&store);
        documents.create(&document("d1", "p1")).unwrap();

        projects.delete("p1").unwrap();

        assert_eq!(documents.get("d1").unwrap(), None);
    }
}
