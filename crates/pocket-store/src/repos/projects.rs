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
}
