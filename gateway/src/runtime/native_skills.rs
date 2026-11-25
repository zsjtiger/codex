//! Skills for local environments, as Codex reads them: a skill is a folder holding a `SKILL.md`, under a
//! root the app-server is pointed at. Remote environments cannot read files the gateway writes, so they
//! keep getting skill instructions in the prompt instead.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use uuid::Uuid;

/// The directory under an environment's `CODEX_HOME` that holds the workspace's skills.
const ROOT_DIR: &str = "gateway-skills";

/// A skill as the gateway keeps it: its id, name, description, and the instructions it adds.
pub struct SkillRow {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub instructions: String,
}

/// A name that is safe to use as a folder: no separators, no hidden or parent folder.
pub fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name.trim() == name
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\0'])
}

/// The `SKILL.md` a skill is written as: name and description in front, then its instructions.
pub fn skill_file(name: &str, description: &str, instructions: &str) -> String {
    // The front matter is one line per field, so line breaks in the description are folded.
    format!(
        "---\nname: {name}\ndescription: {}\n---\n\n{instructions}\n",
        description.replace(['\r', '\n'], " ")
    )
}

/// Where a skill's `SKILL.md` is, under the root.
pub fn skill_path(root: &Path, name: &str) -> PathBuf {
    root.join(name).join("SKILL.md")
}

/// Writes the enabled skills into the environment's root, so Codex finds them, and removes the folders of
/// skills that are no longer there. Files already holding the right text are left alone. Returns the root.
pub fn sync(home: &Path, skills: &[SkillRow]) -> std::io::Result<PathBuf> {
    let root = home.join(ROOT_DIR);
    fs::create_dir_all(&root)?;
    let mut kept = HashSet::new();
    for skill in skills.iter().filter(|skill| safe_name(&skill.name)) {
        let dir = root.join(&skill.name);
        fs::create_dir_all(&dir)?;
        let content = skill_file(&skill.name, &skill.description, &skill.instructions);
        let path = dir.join("SKILL.md");
        if fs::read_to_string(&path).ok().as_deref() != Some(content.as_str()) {
            fs::write(&path, content)?;
        }
        kept.insert(skill.name.clone());
    }
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type()?.is_dir() && !kept.contains(&name) {
            fs::remove_dir_all(entry.path())?;
        }
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(name: &str, instructions: &str) -> SkillRow {
        SkillRow {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: "说明\n分两行".to_string(),
            instructions: instructions.to_string(),
        }
    }

    #[test]
    fn enabled_skills_are_written_and_removed_ones_are_not_kept() {
        let home = std::env::temp_dir().join(format!("native-skills-{}", Uuid::new_v4()));
        let root = sync(&home, &[skill("论文写作", "v1"), skill("周报", "x")]).unwrap();
        assert_eq!(fs::read_to_string(skill_path(&root, "论文写作")).unwrap(), skill_file("论文写作", "说明 分两行", "v1"));

        // The next sync has one skill left, with new text: the other folder goes, the file changes.
        let root = sync(&home, &[skill("论文写作", "v2")]).unwrap();
        assert!(fs::read_to_string(skill_path(&root, "论文写作")).unwrap().contains("v2"));
        assert!(!root.join("周报").exists());
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn names_that_are_not_folder_names_are_skipped() {
        assert!(!safe_name("../etc"));
        assert!(!safe_name("a/b"));
        assert!(!safe_name(".hidden"));
        assert!(!safe_name(" padded "));
        assert!(safe_name("论文写作"));
        let home = std::env::temp_dir().join(format!("native-skills-{}", Uuid::new_v4()));
        let root = sync(&home, &[skill("../escape", "x")]).unwrap();
        assert!(!home.join("escape").exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        let _ = fs::remove_dir_all(&home);
    }
}
