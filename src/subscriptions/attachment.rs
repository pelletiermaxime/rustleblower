//! Local list-rule drafts and bounded inspection of confirmed daemon inventory.
use super::model::{Publication, Row};
use crate::{
    protocol::{Operator, Rule},
    rules::{
        self,
        transfer::{self, ExportScope},
    },
};
use anyhow::{Result, ensure};
use std::path::{Path, PathBuf};

pub const PAGE: usize = 16;
pub struct AttachedPage {
    pub rules: Vec<Rule>,
    pub total: usize,
    pub offset: usize,
}
pub fn directory(publication: &Publication) -> Result<PathBuf, String> {
    Path::new(&publication.file)
        .parent()
        .map(Path::to_owned)
        .ok_or_else(|| "Published list has no directory".into())
}
pub fn draft(row: &Row) -> Result<Rule, String> {
    if row.removed {
        return Err("Restore this subscription before creating a rule".into());
    }
    let publication = row
        .publication
        .as_ref()
        .ok_or("Refresh the current URL and format before creating a list rule")?;
    if publication.format != row.spec.format || publication.url != row.spec.url {
        return Err("Subscription URL/format changed; refresh before reviewing a new rule".into());
    }
    let mut rule = directory_draft(publication.format.operand(), &directory(publication)?)?;
    rule.name = format!(
        "rustleblower-list-{}-{}",
        row.id,
        publication.format.directory()
    );
    rule.description = format!(
        "Subscription: {} ({}). Local-user scope; directory is on this local daemon's machine.",
        row.spec.name,
        publication.format.key()
    );
    rules::document::validate_edit(&rule)?;
    Ok(rule)
}
pub fn directory_draft(operand: &str, directory: &Path) -> Result<Rule, String> {
    if !directory.is_absolute()
        || directory.as_os_str().len() > 4096
        || directory.to_string_lossy().contains('\0')
        || ![
            "lists.domains",
            "lists.ips",
            "lists.nets",
            "lists.domains_regexp",
            "lists.hash.md5",
        ]
        .contains(&operand)
    {
        return Err("Invalid local list-directory target".into());
    }
    Ok(Rule {
        name: "rustleblower-list".into(),
        enabled: true,
        action: "deny".into(),
        duration: "always".into(),
        created: crate::history::now_seconds(),
        operator: Some(Operator {
            r#type: "list".into(),
            operand: "list".into(),
            list: vec![
                Operator {
                    r#type: "simple".into(),
                    operand: "user.id".into(),
                    data: rustix::process::getuid().as_raw().to_string(),
                    sensitive: true,
                    ..Default::default()
                },
                Operator {
                    r#type: "lists".into(),
                    operand: operand.into(),
                    data: directory.to_string_lossy().into_owned(),
                    sensitive: false,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        ..Default::default()
    })
}
pub fn references(rule: &Rule, publication: &Publication) -> bool {
    let Ok(directory) = directory(publication) else {
        return false;
    };
    references_directory(rule, publication.format.operand(), &directory)
}
pub fn references_directory(rule: &Rule, operand: &str, directory: &Path) -> bool {
    fn matches(op: &Operator, operand: &str, directory: &Path) -> bool {
        (op.r#type == "lists" && op.operand == operand && Path::new(&op.data) == directory)
            || op
                .list
                .iter()
                .any(|child| matches(child, operand, directory))
    }
    rule.operator
        .as_ref()
        .is_some_and(|op| matches(op, operand, directory))
}
pub fn page(
    database: &Path,
    node: &str,
    publication: &Publication,
    offset: usize,
) -> Result<AttachedPage> {
    page_for_directory(
        database,
        node,
        publication.format.operand(),
        &directory(publication).map_err(anyhow::Error::msg)?,
        offset,
    )
}
pub fn page_for_directory(
    database: &Path,
    node: &str,
    operand: &str,
    directory: &Path,
    offset: usize,
) -> Result<AttachedPage> {
    ensure!(
        node.len() <= 128 && offset <= rules::MAX_RULES,
        "Invalid attached-rule query"
    );
    let mut result = AttachedPage {
        rules: Vec::new(),
        total: 0,
        offset,
    };
    let mut bytes = 0;
    transfer::visit(
        database,
        &ExportScope {
            query: rules::Query {
                node: node.into(),
                ..Default::default()
            },
            names: None,
        },
        |rule, awaiting| {
            if !awaiting && references_directory(rule, operand, directory) {
                if result.total >= offset && result.rules.len() < PAGE {
                    bytes += rules::memory_bytes(rule);
                    ensure!(
                        bytes <= 512 * 1024,
                        "Attached-rule page exceeds 512 KiB; inspect a smaller selection in Application rules"
                    );
                    result.rules.push(rule.clone());
                }
                result.total += 1;
            }
            Ok(())
        },
    )?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_list_directory_scope_uses_the_kernel_real_uid() {
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let uid = status
            .lines()
            .find(|line| line.starts_with("Uid:"))
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap();
        let rule = directory_draft("lists.domains", Path::new("/tmp/owned-list-fixture")).unwrap();
        let operator = rule.operator.unwrap();
        assert_eq!(operator.r#type, "list");
        assert_eq!(operator.list.len(), 2);
        assert_eq!(operator.list[0].operand, "user.id");
        assert_eq!(operator.list[0].data, uid);
        assert!(operator.list[0].sensitive);
        assert_eq!(operator.list[1].operand, "lists.domains");
        assert_eq!(operator.list[1].data, "/tmp/owned-list-fixture");
        println!("Kernel real UID {uid}: exact UID AND watched-directory draft verified");
    }
}
