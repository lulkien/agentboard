use super::list::TaskHead;
use super::open::{Board, Error, Result, read_txn, write_metadata};
use crate::domain::{self, NewProject, Project};

fn key(id: u64) -> [u8; 8] {
    id.to_be_bytes()
}

impl Board {
    pub fn delete_project(&self, id: u64) -> Result<Project> {
        let mut txn = self.env.write_txn()?;
        self.checked_metadata(&txn)?;
        let project_key = key(id);
        let raw = self
            .projects
            .get(&txn, &project_key)?
            .ok_or(Error::MissingProject(id))?;
        let project: Project = serde_json::from_slice(raw)?;
        let mut count = 0_u64;
        for entry in self.tasks.iter(&txn)? {
            let (_, raw) = entry?;
            #[cfg(test)]
            super::note_task_read();
            let task: TaskHead<'_> = serde_json::from_slice(raw)?;
            if task.project_id == Some(id) {
                count += 1;
            }
        }
        if count > 0 {
            return Err(Error::ProjectInUse { id, count });
        }
        self.projects.delete(&mut txn, &project_key)?;
        txn.commit()?;
        Ok(project)
    }

    pub fn add_project(&self, input: NewProject) -> Result<Project> {
        domain::validate_project(&input.project_name, &input.project_path)?;
        let mut txn = self.env.write_txn()?;
        let mut metadata = self.checked_metadata(&txn)?;
        let id = metadata.next_project_id;
        let project = Project {
            project_id: id,
            project_name: input.project_name,
            project_path: input.project_path,
        };
        self.projects
            .put(&mut txn, &key(id), &serde_json::to_vec(&project)?)?;
        metadata.next_project_id = id.checked_add(1).ok_or(Error::CounterOverflow)?;
        write_metadata(self.meta, &mut txn, &metadata)?;
        txn.commit()?;
        Ok(project)
    }

    pub fn get_project(&self, id: u64) -> Result<Option<Project>> {
        let txn = read_txn(&self.env)?;
        self.projects
            .get(&txn, &key(id))?
            .map(serde_json::from_slice)
            .transpose()
            .map_err(Into::into)
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let txn = read_txn(&self.env)?;
        self.projects
            .iter(&txn)?
            .map(|entry| {
                let (_, value) = entry?;
                serde_json::from_slice(value).map_err(Error::from)
            })
            .collect()
    }

    pub fn update_project(
        &self,
        id: u64,
        project_name: Option<String>,
        project_path: Option<String>,
    ) -> Result<Project> {
        if project_name.is_none() && project_path.is_none() {
            return Err(domain::Error::EmptyProjectPatch.into());
        }
        let mut txn = self.env.write_txn()?;
        self.checked_metadata(&txn)?;
        let project_key = key(id);
        let raw = self
            .projects
            .get(&txn, &project_key)?
            .ok_or(Error::MissingProject(id))?;
        let mut project: Project = serde_json::from_slice(raw)?;
        if let Some(name) = project_name {
            project.project_name = name;
        }
        if let Some(path) = project_path {
            project.project_path = path;
        }
        domain::validate_project(&project.project_name, &project.project_path)?;
        self.projects
            .put(&mut txn, &project_key, &serde_json::to_vec(&project)?)?;
        txn.commit()?;
        Ok(project)
    }
}
