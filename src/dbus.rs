use crate::error::AppError;
use crate::state::ProviderSummary;
use std::collections::HashMap;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const WATCHER_DESTINATION: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_INTERFACE: &str = "org.kde.StatusNotifierWatcher";
const ITEM_INTERFACE: &str = "org.kde.StatusNotifierItem";
const MENU_INTERFACE: &str = "com.canonical.dbusmenu";

type RawLayout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

#[derive(Clone, Debug, PartialEq, Eq)]
struct StatusNotifierAddress {
    destination: String,
    path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MenuNode {
    id: i32,
    label: Option<String>,
    toggle_state: Option<i32>,
    enabled: bool,
    children: Vec<MenuNode>,
}

pub struct TrayClient {
    connection: Connection,
    destination: String,
    menu_path: String,
}

impl TrayClient {
    pub fn connect() -> Result<Self, AppError> {
        let connection = Connection::session().map_err(gui_unavailable)?;
        let address = find_cc_switch_item(&connection)?;
        let item = Proxy::new(
            &connection,
            address.destination.as_str(),
            address.path.as_str(),
            ITEM_INTERFACE,
        )
        .map_err(gui_unavailable)?;
        let menu_path: OwnedObjectPath = item.get_property("Menu").map_err(gui_unavailable)?;

        Ok(Self {
            connection,
            destination: address.destination.clone(),
            menu_path: menu_path.to_string(),
        })
    }

    pub fn activate(
        &self,
        providers: &[ProviderSummary],
        provider_id: &str,
    ) -> Result<(), AppError> {
        let menu = self.read_menu()?;
        let target = map_target_item(&menu, providers, provider_id)?;
        if !target.enabled {
            return Err(AppError::Gui(
                "the selected provider is disabled in the CC Switch tray menu".into(),
            ));
        }

        let proxy = self.menu_proxy()?;
        let click_data = Value::new(0i32);
        proxy
            .call::<_, _, ()>("Event", &(target.id, "clicked", click_data, 0u32))
            .map_err(gui_error)
    }

    pub fn is_selected(
        &self,
        providers: &[ProviderSummary],
        provider_id: &str,
    ) -> Result<bool, AppError> {
        let menu = self.read_menu()?;
        Ok(map_target_item(&menu, providers, provider_id)?.toggle_state == Some(1))
    }

    fn menu_proxy(&self) -> Result<Proxy<'_>, AppError> {
        Proxy::new(
            &self.connection,
            self.destination.as_str(),
            self.menu_path.as_str(),
            MENU_INTERFACE,
        )
        .map_err(gui_error)
    }

    fn read_menu(&self) -> Result<MenuNode, AppError> {
        let proxy = self.menu_proxy()?;
        let (_revision, layout): (u32, RawLayout) = proxy
            .call(
                "GetLayout",
                &(0i32, -1i32, vec!["label", "toggle-state", "enabled"]),
            )
            .map_err(gui_error)?;
        parse_layout(layout).map_err(AppError::Gui)
    }
}

fn find_cc_switch_item(connection: &Connection) -> Result<StatusNotifierAddress, AppError> {
    let watcher = Proxy::new(
        connection,
        WATCHER_DESTINATION,
        WATCHER_PATH,
        WATCHER_INTERFACE,
    )
    .map_err(gui_unavailable)?;
    let items: Vec<String> = watcher
        .get_property("RegisteredStatusNotifierItems")
        .map_err(gui_unavailable)?;

    for item in items {
        let Some(address) = parse_status_notifier_address(&item) else {
            continue;
        };
        let Ok(proxy) = Proxy::new(
            connection,
            address.destination.as_str(),
            address.path.as_str(),
            ITEM_INTERFACE,
        ) else {
            continue;
        };
        if proxy.get_property::<String>("Title").ok().as_deref() == Some("cc-switch") {
            return Ok(address.clone());
        }
    }

    Err(AppError::GuiUnavailable(
        "no StatusNotifierItem titled \"cc-switch\" is registered".into(),
    ))
}

fn parse_status_notifier_address(item: &str) -> Option<StatusNotifierAddress> {
    if let Some(index) = item.find('/') {
        let destination = &item[..index];
        let path = &item[index..];
        if destination.is_empty() || path.is_empty() {
            return None;
        }
        return Some(StatusNotifierAddress {
            destination: destination.to_owned(),
            path: path.to_owned(),
        });
    }
    (!item.is_empty()).then(|| StatusNotifierAddress {
        destination: item.to_owned(),
        path: "/StatusNotifierItem".into(),
    })
}

fn parse_layout((id, properties, children): RawLayout) -> Result<MenuNode, String> {
    let label = property_string(&properties, "label")?;
    let toggle_state = property_i32(&properties, "toggle-state")?;
    let enabled = property_bool(&properties, "enabled")?.unwrap_or(true);
    let children = children
        .into_iter()
        .map(|child| {
            let structure = child
                .downcast_ref::<&zbus::zvariant::Structure<'_>>()
                .map_err(|error| format!("invalid menu child: {error}"))?;
            let fields = structure.fields();
            if fields.len() != 3 {
                return Err(format!(
                    "invalid menu child: expected 3 fields, found {}",
                    fields.len()
                ));
            }
            let child_id = fields[0]
                .downcast_ref::<i32>()
                .map_err(|error| format!("invalid menu item id: {error}"))?;
            let child_properties = fields[1]
                .downcast_ref::<&zbus::zvariant::Dict<'_, '_>>()
                .map_err(|error| format!("invalid menu item properties: {error}"))?;
            let child_children = fields[2]
                .downcast_ref::<&zbus::zvariant::Array<'_>>()
                .map_err(|error| format!("invalid menu item children: {error}"))?;
            let properties = dict_to_map(child_properties)?;
            let children = child_children
                .inner()
                .iter()
                .map(OwnedValue::try_from)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("invalid nested menu item: {error}"))?;
            parse_layout((child_id, properties, children))
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(MenuNode {
        id,
        label,
        toggle_state,
        enabled,
        children,
    })
}

fn dict_to_map(
    dictionary: &zbus::zvariant::Dict<'_, '_>,
) -> Result<HashMap<String, OwnedValue>, String> {
    dictionary
        .iter()
        .map(|(key, value)| {
            let key = key
                .downcast_ref::<&str>()
                .map_err(|error| format!("invalid menu property name: {error}"))?;
            let value = OwnedValue::try_from(value)
                .map_err(|error| format!("invalid menu property value: {error}"))?;
            Ok((key.to_owned(), value))
        })
        .collect()
}

fn property_string(
    properties: &HashMap<String, OwnedValue>,
    key: &str,
) -> Result<Option<String>, String> {
    properties
        .get(key)
        .map(|value| {
            value
                .downcast_ref::<&str>()
                .map(str::to_owned)
                .map_err(|error| format!("invalid {key} property: {error}"))
        })
        .transpose()
}

fn property_i32(
    properties: &HashMap<String, OwnedValue>,
    key: &str,
) -> Result<Option<i32>, String> {
    properties
        .get(key)
        .map(|value| {
            value
                .downcast_ref::<i32>()
                .map_err(|error| format!("invalid {key} property: {error}"))
        })
        .transpose()
}

fn property_bool(
    properties: &HashMap<String, OwnedValue>,
    key: &str,
) -> Result<Option<bool>, String> {
    properties
        .get(key)
        .map(|value| {
            value
                .downcast_ref::<bool>()
                .map_err(|error| format!("invalid {key} property: {error}"))
        })
        .transpose()
}

fn map_target_item<'a>(
    root: &'a MenuNode,
    providers: &[ProviderSummary],
    provider_id: &str,
) -> Result<&'a MenuNode, AppError> {
    let codex = root
        .children
        .iter()
        .find(|node| {
            node.label
                .as_deref()
                .is_some_and(|label| label == "Codex" || label.starts_with("Codex · "))
        })
        .ok_or_else(|| AppError::Gui("Codex submenu is not visible in the tray menu".into()))?;
    let items = codex
        .children
        .iter()
        .filter(|node| node.toggle_state.is_some())
        .collect::<Vec<_>>();
    if items.len() != providers.len() {
        return Err(AppError::Gui(format!(
            "tray/provider count mismatch (tray {}, database {})",
            items.len(),
            providers.len()
        )));
    }

    for (item, provider) in items.iter().zip(providers) {
        let label = item.label.as_deref().unwrap_or_default();
        let blocked_label = format!("{} \u{26D4}", provider.name);
        if label != provider.name && label != blocked_label {
            return Err(AppError::Gui(
                "tray provider order does not match the supported CC Switch layout".into(),
            ));
        }
    }

    let index = providers
        .iter()
        .position(|provider| provider.id == provider_id)
        .ok_or_else(|| AppError::ProviderIdNotFound(provider_id.to_owned()))?;
    Ok(items[index])
}

fn gui_unavailable(error: impl std::fmt::Display) -> AppError {
    AppError::GuiUnavailable(error.to_string())
}

fn gui_error(error: impl std::fmt::Display) -> AppError {
    AppError::Gui(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: i32, label: &str, toggle_state: Option<i32>) -> MenuNode {
        MenuNode {
            id,
            label: Some(label.into()),
            toggle_state,
            enabled: true,
            children: Vec::new(),
        }
    }

    fn provider(id: &str, name: &str) -> ProviderSummary {
        ProviderSummary {
            id: id.into(),
            name: name.into(),
            current: false,
        }
    }

    #[test]
    fn parses_status_notifier_addresses() {
        assert_eq!(
            parse_status_notifier_address(":1.62/org/example/Item"),
            Some(StatusNotifierAddress {
                destination: ":1.62".into(),
                path: "/org/example/Item".into()
            })
        );
        assert_eq!(
            parse_status_notifier_address("org.example.Item"),
            Some(StatusNotifierAddress {
                destination: "org.example.Item".into(),
                path: "/StatusNotifierItem".into()
            })
        );
    }

    #[test]
    fn maps_duplicate_names_by_database_order_and_id() {
        let providers = vec![provider("first", "Same"), provider("second", "Same")];
        let root = MenuNode {
            id: 0,
            label: None,
            toggle_state: None,
            enabled: true,
            children: vec![MenuNode {
                id: 1,
                label: Some("Codex · Same".into()),
                toggle_state: None,
                enabled: true,
                children: vec![node(10, "Same", Some(1)), node(11, "Same", Some(0))],
            }],
        };

        assert_eq!(map_target_item(&root, &providers, "second").unwrap().id, 11);
    }

    #[test]
    fn rejects_layout_order_mismatch() {
        let providers = vec![provider("a", "A"), provider("b", "B")];
        let root = MenuNode {
            id: 0,
            label: None,
            toggle_state: None,
            enabled: true,
            children: vec![MenuNode {
                id: 1,
                label: Some("Codex".into()),
                toggle_state: None,
                enabled: true,
                children: vec![node(10, "B", Some(0)), node(11, "A", Some(1))],
            }],
        };

        assert!(matches!(
            map_target_item(&root, &providers, "a"),
            Err(AppError::Gui(_))
        ));
    }
}
