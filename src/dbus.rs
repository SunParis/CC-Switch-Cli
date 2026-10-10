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
    separator: bool,
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
                &(
                    0i32,
                    -1i32,
                    vec!["label", "toggle-state", "enabled", "type"],
                ),
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
    let separator = property_string(&properties, "type")?.as_deref() == Some("separator");
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
        separator,
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
    // In 4.x, project checkboxes follow a separator in the same submenu.
    let items = codex
        .children
        .iter()
        .take_while(|node| !node.separator)
        .filter(|node| node.toggle_state.is_some())
        .collect::<Vec<_>>();
    let index = providers
        .iter()
        .position(|provider| provider.id == provider_id)
        .ok_or_else(|| AppError::ProviderIdNotFound(provider_id.to_owned()))?;

    // The full list disambiguates duplicate names by the upstream provider order.
    if items.len() == providers.len() {
        if items
            .iter()
            .zip(providers)
            .enumerate()
            .all(|(index, (item, _))| {
                label_matches(item.label.as_deref().unwrap_or_default(), providers, index)
            })
        {
            return Ok(items[index]);
        }
        return Err(AppError::Gui(
            "tray provider order does not match the supported CC Switch layout".into(),
        ));
    }

    // Routing/aggregation can hide providers. Only select an unambiguous label;
    // never guess a duplicate's identity from its position in a partial list.
    let matches = items
        .iter()
        .filter(|item| label_matches(item.label.as_deref().unwrap_or_default(), providers, index))
        .collect::<Vec<_>>();
    if let [target] = matches.as_slice() {
        let label = target.label.as_deref().unwrap_or_default();
        let possible = providers
            .iter()
            .filter(|provider| {
                let base = tray_name(&provider.name);
                label == provider.name
                    || label == base
                    || label.starts_with(&format!("{base} · "))
                    || label.starts_with(&format!("{base} #"))
                    || label.split_once(". ").is_some_and(|(number, rest)| {
                        number.parse::<usize>().is_ok() && rest == base
                    })
            })
            .count();
        if possible == 1 {
            return Ok(target);
        }
    }
    Err(AppError::Gui(
        "the target provider is unavailable or ambiguous in the CC Switch tray menu".into(),
    ))
}

fn tray_name(name: &str) -> String {
    let name = name.trim();
    if name.chars().count() > 32 {
        name.chars().take(31).chain(std::iter::once('…')).collect()
    } else {
        name.to_owned()
    }
}

fn label_matches(label: &str, providers: &[ProviderSummary], index: usize) -> bool {
    let name = &providers[index].name;
    let base = tray_name(name);
    if label == name || label == base || label == format!("{name} \u{26D4}") {
        return true;
    }
    let duplicate = providers
        .iter()
        .filter(|p| p.name.trim() == name.trim())
        .count()
        > 1;
    if duplicate {
        let ordinal = providers[..=index]
            .iter()
            .filter(|p| p.name.trim() == name.trim())
            .count();
        if label == format!("{base} #{ordinal}") || label.starts_with(&format!("{base} · ")) {
            return true;
        }
    }
    // Auto-failover rows are numbered and disabled in 4.0.6.
    label
        .split_once(". ")
        .is_some_and(|(number, rest)| number.parse::<usize>().is_ok() && rest == base)
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

    #[test]
    #[ignore = "read-only check requiring a running CC Switch and desktop session bus"]
    fn live_gui_current_provider_matches_stored_state() {
        let reader =
            crate::state::StateReader::open(crate::state::Paths::discover().unwrap()).unwrap();
        let providers = reader.providers().unwrap();
        let current = providers.iter().find(|provider| provider.current).unwrap();
        let tray = TrayClient::connect().unwrap();
        assert!(tray.is_selected(&providers, &current.id).unwrap());
    }

    fn node(id: i32, label: &str, toggle_state: Option<i32>) -> MenuNode {
        MenuNode {
            id,
            label: Some(label.into()),
            toggle_state,
            enabled: true,
            separator: false,
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
            separator: false,
            children: vec![MenuNode {
                id: 1,
                label: Some("Codex · Same".into()),
                toggle_state: None,
                enabled: true,
                separator: false,
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
            separator: false,
            children: vec![MenuNode {
                id: 1,
                label: Some("Codex".into()),
                toggle_state: None,
                enabled: true,
                separator: false,
                children: vec![node(10, "B", Some(0)), node(11, "A", Some(1))],
            }],
        };

        assert!(matches!(
            map_target_item(&root, &providers, "a"),
            Err(AppError::Gui(_))
        ));
    }

    fn menu(items: Vec<MenuNode>) -> MenuNode {
        let mut codex = node(1, "Codex · Routing · Current", None);
        codex.children = items;
        let mut root = node(0, "", None);
        root.children = vec![codex];
        root
    }

    #[test]
    fn modern_menu_truncates_names_and_excludes_project_checkboxes() {
        let name = "abcdefghijklmnopqrstuvwxyz0123456789";
        let providers = vec![provider("one", name), provider("two", "Other")];
        let mut separator = node(4, "", None);
        separator.separator = true;
        let root = menu(vec![
            node(2, "Routing", None),
            node(10, "abcdefghijklmnopqrstuvwxyz01234…", Some(1)),
            node(11, "Other", Some(0)),
            separator,
            node(12, "Other", Some(1)),
        ]);
        assert_eq!(map_target_item(&root, &providers, "two").unwrap().id, 11);
    }

    #[test]
    fn modern_menu_disambiguates_duplicate_names_with_ordinals() {
        let providers = vec![provider("one", "Same"), provider("two", "Same")];
        let root = menu(vec![
            node(10, "Same #1", Some(0)),
            node(11, "Same #2", Some(1)),
        ]);
        assert_eq!(map_target_item(&root, &providers, "two").unwrap().id, 11);
    }

    #[test]
    fn filtered_menu_resolves_unique_provider() {
        let providers = vec![
            provider("one", "One"),
            provider("two", "Two"),
            provider("three", "Three"),
        ];
        let root = menu(vec![node(10, "One", Some(1)), node(12, "Three", Some(0))]);
        assert_eq!(map_target_item(&root, &providers, "three").unwrap().id, 12);
    }

    #[test]
    fn filtered_menu_does_not_guess_duplicate_identity() {
        let providers = vec![provider("one", "Same"), provider("two", "Same")];
        let root = menu(vec![node(10, "Same #1", Some(0))]);
        assert!(matches!(
            map_target_item(&root, &providers, "one"),
            Err(AppError::Gui(_))
        ));
    }

    #[test]
    fn failover_menu_keeps_provider_disabled() {
        let providers = vec![provider("one", "One"), provider("two", "Two")];
        let mut item = node(10, "1. Two", Some(1));
        item.enabled = false;
        let root = menu(vec![item]);
        assert!(!map_target_item(&root, &providers, "two").unwrap().enabled);
    }
}
