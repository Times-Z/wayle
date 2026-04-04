use crate::{PluginDropdownItem, PluginTypedRowAction};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    EmptyLabel,
    EmptyCommand,
    EmptyText,
    EmptyPayload,
    InvalidUrlScheme,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyLabel => f.write_str("label must not be empty"),
            Self::EmptyCommand => f.write_str("command must not be empty"),
            Self::EmptyText => f.write_str("text must not be empty"),
            Self::EmptyPayload => f.write_str("custom payload must not be empty"),
            Self::InvalidUrlScheme => f.write_str("url must start with http:// or https://"),
        }
    }
}

impl std::error::Error for ValidationError {}

pub fn validate_label(label: &str) -> Result<(), ValidationError> {
    if label.trim().is_empty() {
        return Err(ValidationError::EmptyLabel);
    }
    Ok(())
}

pub fn validate_command(command: &str) -> Result<(), ValidationError> {
    if command.trim().is_empty() {
        return Err(ValidationError::EmptyCommand);
    }
    Ok(())
}

pub fn validate_copy_text(text: &str) -> Result<(), ValidationError> {
    if text.trim().is_empty() {
        return Err(ValidationError::EmptyText);
    }
    Ok(())
}

pub fn validate_custom_payload(payload: &str) -> Result<(), ValidationError> {
    if payload.trim().is_empty() {
        return Err(ValidationError::EmptyPayload);
    }
    Ok(())
}

pub fn validate_url(url: &str) -> Result<(), ValidationError> {
    let trimmed = url.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Ok(());
    }
    Err(ValidationError::InvalidUrlScheme)
}

pub fn validate_typed_action(action: &PluginTypedRowAction) -> Result<(), ValidationError> {
    match action {
        PluginTypedRowAction::RunCommand { command } => validate_command(command),
        PluginTypedRowAction::OpenUrl { url } => validate_url(url),
        PluginTypedRowAction::CopyText { text } => validate_copy_text(text),
        PluginTypedRowAction::Custom { payload } => validate_custom_payload(payload),
        PluginTypedRowAction::RefreshNow => Ok(()),
    }
}

pub fn validate_dropdown_item(item: &PluginDropdownItem) -> Result<(), ValidationError> {
    let label_empty = item.label.trim().is_empty();

    // Separator rows are represented by empty label + no description + no action.
    if label_empty && item.description.is_none() && item.action.is_none() {
        return Ok(());
    }

    if label_empty {
        return Err(ValidationError::EmptyLabel);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{PluginDropdownItem, PluginTypedRowAction};

    use super::{
        ValidationError, validate_command, validate_copy_text, validate_custom_payload,
        validate_dropdown_item, validate_label, validate_typed_action, validate_url,
    };

    #[test]
    fn rejects_empty_label() {
        let error = validate_label("   ").expect_err("empty label should fail");
        assert_eq!(error, ValidationError::EmptyLabel);
    }

    #[test]
    fn rejects_empty_command() {
        let error = validate_command("   ").expect_err("empty command should fail");
        assert_eq!(error, ValidationError::EmptyCommand);
    }

    #[test]
    fn rejects_empty_copy_text() {
        let error = validate_copy_text("   ").expect_err("empty text should fail");
        assert_eq!(error, ValidationError::EmptyText);
    }

    #[test]
    fn rejects_empty_custom_payload() {
        let error = validate_custom_payload("   ").expect_err("empty payload should fail");
        assert_eq!(error, ValidationError::EmptyPayload);
    }

    #[test]
    fn rejects_invalid_url_scheme() {
        let error = validate_url("file:///tmp/a").expect_err("should reject file scheme");
        assert_eq!(error, ValidationError::InvalidUrlScheme);
    }

    #[test]
    fn accepts_valid_run_command_action() {
        let action = PluginTypedRowAction::RunCommand {
            command: String::from("echo ok"),
        };
        validate_typed_action(&action).expect("command action should be valid");
    }

    #[test]
    fn accepts_separator_dropdown_item() {
        let item = PluginDropdownItem {
            label: String::new(),
            description: None,
            action: None,
        };
        validate_dropdown_item(&item).expect("separator row should be valid");
    }

    #[test]
    fn accepts_other_typed_actions() {
        validate_typed_action(&PluginTypedRowAction::OpenUrl {
            url: String::from("https://example.org"),
        })
        .expect("https url should be valid");

        validate_typed_action(&PluginTypedRowAction::CopyText {
            text: String::from("hello"),
        })
        .expect("copy text should be valid");

        validate_typed_action(&PluginTypedRowAction::Custom {
            payload: String::from("collapse:system"),
        })
        .expect("custom payload should be valid");

        validate_typed_action(&PluginTypedRowAction::RefreshNow)
            .expect("refresh action should be valid");
    }

    #[test]
    fn rejects_non_separator_empty_label_row() {
        let item = PluginDropdownItem {
            label: String::new(),
            description: Some(String::from("desc")),
            action: None,
        };
        let error = validate_dropdown_item(&item).expect_err("invalid row should fail");
        assert_eq!(error, ValidationError::EmptyLabel);
    }
}
