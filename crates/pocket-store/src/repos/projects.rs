use rusqlite::params;

use crate::Store;

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectRow {
    pub id: String,
    pub name: String,
    pub instructions: Option<String>,
    pub created_at: i64,
}

pub struct ProjectsRepo<'a> {
    store: &'a Store,
}

impl<'a> ProjectsRepo<'a> {
    pub fn new(store: &'a Store) -> Self {
        Self { store }
    }

    pub fn create(&self, row: &ProjectRow) -> rusqlite::Result<()> {
        self.store.writer().execute(
            "INSERT INTO projects (id, name, instructions, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![row.id, row.name, row.instructions, row.created_at],
        )?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> rusqlite::Result<Option<ProjectRow>> {
        self.store
            .reader()
            .query_row(
                "SELECT id, name, instructions, created_at FROM projects WHERE id = ?1",
                [id],
                Self::map_row,
            )
            .map(Some)
            .or_else(|e| {
                if e == rusqlite::Error::QueryReturnedNoRows {
                    Ok(None)
                } else {
                    Err(e)
                }
            })
    }

    pub fn update_name(&self, id: &str, name: &str) -> rusqlite::Result<()> {
        self.store.writer().execute(
            "UPDATE projects SET name = ?2 WHERE id = ?1",
            params![id, name],
        )?;
        Ok(())
    }

    /// `None` clears the instructions; the column is nullable and an empty
    /// string would otherwise be prepended to every prompt as a blank line.
    pub fn update_instructions(&self, id: &str, instructions: Option<&str>) -> rusqlite::Result<()> {
        self.store.writer().execute(
            "UPDATE projects SET instructions = ?2 WHERE id = ?1",
            params![id, instructions],
        )?;
        Ok(())
    }

    pub fn list_all(&self) -> rusqlite::Result<Vec<ProjectRow>> {
        let conn = self.store.reader();
        let mut stmt = conn.prepare(
            "SELECT id, name, instructions, created_at FROM projects ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], Self::map_row)?;
        rows.collect()
    }

    pub fn delete(&self, id: &str) -> rusqlite::Result<()> {
        self.store
            .writer()
            .execute("DELETE FROM projects WHERE id = ?1", [id])?;
        Ok(())
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<ProjectRow> {
        Ok(ProjectRow {
            id: row.get(0)?,
            name: row.get(1)?,
            instructions: row.get(2)?,
            created_at: row.get(3)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crud_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let repo = ProjectsRepo::new(&store);
        repo.create(&ProjectRow {
            id: "p1".into(),
            name: "Research".into(),
            instructions: None,
            created_at: 1,
        })
        .unwrap();
        assert_eq!(repo.get("p1").unwrap().unwrap().name, "Research");
        repo.update_name("p1", "Renamed").unwrap();
        assert_eq!(repo.get("p1").unwrap().unwrap().name, "Renamed");
        repo.delete("p1").unwrap();
        assert_eq!(repo.get("p1").unwrap(), None);
    }

    #[test]
    fn instructions_round_trip_and_clear() {
        let store = Store::open_in_memory().unwrap();
        let repo = ProjectsRepo::new(&store);
        repo.create(&ProjectRow {
            id: "p1".into(),
            name: "Research".into(),
            instructions: None,
            created_at: 1,
        })
        .unwrap();

        repo.update_instructions("p1", Some("Answer in British English."))
            .unwrap();
        assert_eq!(
            repo.get("p1").unwrap().unwrap().instructions.as_deref(),
            Some("Answer in British English.")
        );

        // Clearing has to null the column, not store "", or every prompt in
        // the project gains a blank leading line.
        repo.update_instructions("p1", None).unwrap();
        assert_eq!(repo.get("p1").unwrap().unwrap().instructions, None);

        // Instructions and name are independent columns.
        repo.update_instructions("p1", Some("Be terse.")).unwrap();
        repo.update_name("p1", "Renamed").unwrap();
        let row = repo.get("p1").unwrap().unwrap();
        assert_eq!(row.name, "Renamed");
        assert_eq!(row.instructions.as_deref(), Some("Be terse."));
    }

    #[test]
    fn lists_all_created_projects() {
        let store = Store::open_in_memory().unwrap();
        let repo = ProjectsRepo::new(&store);
        repo.create(&ProjectRow {
            id: "p1".into(),
            name: "Research 1".into(),
            instructions: None,
            created_at: 10,
        })
        .unwrap();
        repo.create(&ProjectRow {
            id: "p2".into(),
            name: "Research 2".into(),
            instructions: Some("Instructions".into()),
            created_at: 20,
        })
        .unwrap();
        let list = repo.list_all().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "p2");
        assert_eq!(list[1].id, "p1");
    }
}
