use crate::validators::{
    ValidationError, validate_command, validate_custom_payload, validate_dropdown_item,
    validate_label, validate_url,
};
use crate::{
    PluginDropdown, PluginDropdownItem, PluginRowAction, PluginStyle, PluginTypedRowAction,
    PluginUpdate,
};

pub struct ActionBuilder;

impl ActionBuilder {
    pub fn run_command(command: impl Into<String>) -> Result<PluginRowAction, ValidationError> {
        let command = command.into();
        validate_command(&command)?;
        Ok(PluginRowAction::Typed(PluginTypedRowAction::RunCommand {
            command,
        }))
    }

    pub fn open_url(url: impl Into<String>) -> Result<PluginRowAction, ValidationError> {
        let url = url.into();
        validate_url(&url)?;
        Ok(PluginRowAction::Typed(PluginTypedRowAction::OpenUrl {
            url,
        }))
    }

    pub fn copy_text(text: impl Into<String>) -> Result<PluginRowAction, ValidationError> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(ValidationError::EmptyText);
        }
        Ok(PluginRowAction::Typed(PluginTypedRowAction::CopyText {
            text,
        }))
    }

    pub fn custom(payload: impl Into<String>) -> Result<PluginRowAction, ValidationError> {
        let payload = payload.into();
        validate_custom_payload(&payload)?;
        Ok(PluginRowAction::Typed(PluginTypedRowAction::Custom {
            payload,
        }))
    }

    pub fn refresh_now() -> PluginRowAction {
        PluginRowAction::Typed(PluginTypedRowAction::RefreshNow)
    }
}

pub struct PluginUpdateBuilder {
    label: String,
    tooltip: Option<String>,
    visible: bool,
    dropdown: Option<PluginDropdown>,
    style: Option<PluginStyle>,
}

impl PluginUpdateBuilder {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            tooltip: None,
            visible: true,
            dropdown: None,
            style: None,
        }
    }

    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn dropdown(mut self, dropdown: PluginDropdown) -> Self {
        self.dropdown = Some(dropdown);
        self
    }

    pub fn style(mut self, style: PluginStyle) -> Self {
        self.style = Some(style);
        self
    }

    pub fn build(self) -> Result<PluginUpdate, ValidationError> {
        validate_label(&self.label)?;

        if let Some(dropdown) = &self.dropdown {
            for item in &dropdown.items {
                validate_dropdown_item(item)?;
            }
        }

        Ok(PluginUpdate {
            label: self.label,
            tooltip: self.tooltip,
            visible: self.visible,
            dropdown: self.dropdown,
            style: self.style,
        })
    }
}

#[derive(Default)]
pub struct DropdownBuilder {
    title: Option<String>,
    title_xalign: Option<f32>,
    empty_label: Option<String>,
    items: Vec<PluginDropdownItem>,
}

impl DropdownBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn title_xalign(mut self, xalign: f32) -> Self {
        self.title_xalign = Some(xalign.clamp(0.0, 1.0));
        self
    }

    pub fn empty_label(mut self, empty_label: impl Into<String>) -> Self {
        self.empty_label = Some(empty_label.into());
        self
    }

    pub fn item(mut self, item: PluginDropdownItem) -> Self {
        self.items.push(item);
        self
    }

    pub fn separator(mut self) -> Self {
        self.items.push(PluginDropdownItem {
            label: String::new(),
            description: None,
            action: None,
        });
        self
    }

    pub fn build(self) -> PluginDropdown {
        PluginDropdown {
            title: self.title,
            title_xalign: self.title_xalign,
            empty_label: self.empty_label,
            items: self.items,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::validators::ValidationError;
    use crate::{PluginDropdownItem, PluginRowAction, PluginStyle, PluginTypedRowAction};

    use super::{ActionBuilder, DropdownBuilder, PluginUpdateBuilder};

    #[test]
    fn action_builder_rejects_empty_command() {
        let error = ActionBuilder::run_command("  ").expect_err("empty command should fail");
        assert_eq!(error, ValidationError::EmptyCommand);
    }

    #[test]
    fn action_builder_open_url_and_copy_text() {
        let open_url = ActionBuilder::open_url("https://example.org").expect("valid url");
        assert!(matches!(
            open_url,
            PluginRowAction::Typed(PluginTypedRowAction::OpenUrl { .. })
        ));

        let copy = ActionBuilder::copy_text("hello").expect("non-empty text");
        assert!(matches!(
            copy,
            PluginRowAction::Typed(PluginTypedRowAction::CopyText { .. })
        ));

        let refresh = ActionBuilder::refresh_now();
        assert!(matches!(
            refresh,
            PluginRowAction::Typed(PluginTypedRowAction::RefreshNow)
        ));
    }

    #[test]
    fn action_builder_rejects_invalid_url_and_empty_text() {
        let url_error =
            ActionBuilder::open_url("file:///tmp/a").expect_err("file scheme should be rejected");
        assert_eq!(url_error, ValidationError::InvalidUrlScheme);

        let text_error = ActionBuilder::copy_text("   ").expect_err("empty text should fail");
        assert_eq!(text_error, ValidationError::EmptyText);

        let payload_error = ActionBuilder::custom("   ").expect_err("empty payload should fail");
        assert_eq!(payload_error, ValidationError::EmptyPayload);
    }

    #[test]
    fn action_builder_custom_payload() {
        let custom = ActionBuilder::custom("collapse:system").expect("valid custom payload");
        assert!(matches!(
            custom,
            PluginRowAction::Typed(PluginTypedRowAction::Custom { .. })
        ));
    }

    #[test]
    fn dropdown_builder_creates_separator() {
        let dropdown = DropdownBuilder::new().separator().build();
        assert_eq!(dropdown.items.len(), 1);
        assert!(dropdown.items[0].label.is_empty());
    }

    #[test]
    fn dropdown_builder_sets_title_and_empty_label() {
        let dropdown = DropdownBuilder::new()
            .title("My title")
            .empty_label("Nothing here")
            .build();
        assert_eq!(dropdown.title.as_deref(), Some("My title"));
        assert_eq!(dropdown.empty_label.as_deref(), Some("Nothing here"));
    }

    #[test]
    fn update_builder_success_path() {
        let dropdown = DropdownBuilder::new()
            .item(PluginDropdownItem {
                label: String::from("Section"),
                description: None,
                action: None,
            })
            .build();

        let update = PluginUpdateBuilder::new("ok")
            .tooltip("tip")
            .visible(false)
            .dropdown(dropdown)
            .style(PluginStyle {
                button_class: Some(String::from("btn")),
                dropdown_class: Some(String::from("dd")),
                css: None,
                once: true,
                dropdown_height_percent: None,
            })
            .build()
            .expect("valid update should build");

        assert_eq!(update.label, "ok");
        assert_eq!(update.tooltip.as_deref(), Some("tip"));
        assert!(!update.visible);
        assert!(update.style.is_some());
    }

    #[test]
    fn update_builder_validates_items() {
        let dropdown = DropdownBuilder::new()
            .item(PluginDropdownItem {
                label: String::new(),
                description: Some(String::from("not a separator")),
                action: Some(PluginRowAction::Legacy(String::from("echo hi"))),
            })
            .build();

        let error = PluginUpdateBuilder::new("ok")
            .dropdown(dropdown)
            .build()
            .expect_err("invalid row should fail");
        assert_eq!(error, ValidationError::EmptyLabel);
    }
}
