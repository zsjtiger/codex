-- Files kept with a project. They are added to the context of every new turn in the project.
ALTER TABLE attachments ADD COLUMN project_id uuid REFERENCES projects (id) ON DELETE CASCADE;
CREATE INDEX attachments_project_idx ON attachments (project_id);
