use std::sync::Arc;

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;

use crate::agent::tools::{AskSender, PermCheck, ToolError, check_perm};
use crate::skill::{self, Skill};

pub struct SkillTool {
    pub permission: Option<PermCheck>,
    pub ask_tx: Option<AskSender>,
    skills: Arc<[Skill]>,
}

impl SkillTool {
    pub fn new(
        skills: Arc<[Skill]>,
        permission: Option<PermCheck>,
        ask_tx: Option<AskSender>,
    ) -> Self {
        Self {
            permission,
            ask_tx,
            skills,
        }
    }
}

#[derive(Deserialize)]
pub struct Args {
    name: String,
}

impl Tool for SkillTool {
    const NAME: &'static str = "skill";

    type Error = ToolError;
    type Args = Args;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        let mut description =
            "Load a skill by name to get detailed instructions for a specific task or domain."
                .to_string();

        let list = skill::build_skill_list_description(&self.skills);
        if !list.is_empty() {
            description.push_str(&list);
        }

        ToolDefinition {
            name: "skill".to_string(),
            description,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name of the skill to load"
                    }
                },
                "required": ["name"]
            }),
        }
    }

    async fn call(&self, args: Args) -> Result<String, ToolError> {
        check_perm(&self.permission, &self.ask_tx, "skill", &args.name).await?;

        let Some(skill) = skill::find_skill(&args.name, &self.skills) else {
            return Err(ToolError::Msg(format!(
                "Skill '{}' not found. Available: {}",
                args.name,
                self.skills
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        };

        let mut output = format!("# {}\n", skill.name);
        if !skill.description.is_empty() {
            output.push_str(&format!("\n{}\n\n", skill.description));
        }
        output.push_str(&skill.content);

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use super::*;

    fn make_skills() -> Arc<[Skill]> {
        Arc::from([Skill {
            name: "test-skill".into(),
            description: "A test skill".into(),
            content: "Do the thing.".into(),
            location: PathBuf::from("/tmp"),
        }])
    }

    fn make_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }

    #[test]
    fn test_skill_tool_definition_includes_available_skills() {
        let skills = make_skills();
        let tool = SkillTool::new(skills, None, None);
        let rt = make_runtime();
        let def = rt.block_on(tool.definition(String::new()));
        assert!(def.description.contains("test-skill"));
        assert!(def.description.contains("A test skill"));
    }

    #[test]
    fn test_skill_tool_call_loads_skill() {
        let skills = make_skills();
        let tool = SkillTool::new(skills, None, None);
        let rt = make_runtime();
        let result = rt.block_on(tool.call(Args {
            name: "test-skill".into(),
        }));
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("test-skill"));
        assert!(output.contains("Do the thing."));
    }

    #[test]
    fn test_skill_tool_call_not_found() {
        let skills = make_skills();
        let tool = SkillTool::new(skills, None, None);
        let rt = make_runtime();
        let result = rt.block_on(tool.call(Args {
            name: "missing".into(),
        }));
        assert!(result.is_err());
    }
}
