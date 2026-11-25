//! Strict lookup: a target that matches several nodes names none.
//!
//! Match lists are in document order, never hash order, so `Target::Nth` counts as the UI reads.

use bevy::{
    ecs::{
        entity::Entity,
        hierarchy::{ChildOf, Children},
        name::Name,
        world::{EntityRef, World},
    },
    ui::{ComputedNode, UiGlobalTransform, widget::Text},
};

use super::{Lookup, Target, is_tooling, resolve};
use crate::{
    check::{close_matches, component_id, label_of},
    probe::describe_entity,
};

const MESSAGE_PRECISION: f32 = 10.0;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Matches {
    Nodes(Vec<Entity>),
    Missing(String),
    NotANode(String),
}

pub(super) fn locate(world: &World, target: &Target) -> Lookup {
    match matches(world, target) {
        Matches::Nodes(nodes) => match nodes.as_slice() {
            [one] => Lookup::Found(*one),
            [] => Lookup::Missing(nothing_matches(world, target)),
            several => Lookup::Ambiguous(ambiguous(world, target, several)),
        },
        Matches::Missing(message) => Lookup::Missing(message),
        Matches::NotANode(message) => Lookup::NotANode(message),
    }
}

fn matches(world: &World, target: &Target) -> Matches {
    match target {
        Target::Screen(_) | Target::World(_) => {
            Matches::NotANode(format!("{target:?} is a position, not a widget"))
        }
        Target::App(_) => match resolve(world, target) {
            Ok(resolved) => resolved.widget.map_or_else(
                || Matches::NotANode(format!("{target:?} is a position, not a widget")),
                |widget| Matches::Nodes(vec![widget]),
            ),
            Err(message) => Matches::Missing(message),
        },
        Target::Text(text) => Matches::Nodes(app_nodes(world, |entity| {
            entity.get::<Text>().is_some_and(|shown| shown.0 == *text)
        })),
        Target::Name(name) => Matches::Nodes(app_nodes(world, |entity| {
            entity
                .get::<Name>()
                .is_some_and(|named| named.as_str() == name)
        })),
        Target::Role { role, label } => Matches::Nodes(app_nodes(world, |entity| {
            role.fits(entity)
                && label
                    .as_ref()
                    .is_none_or(|label| label_of(world, entity.id()) == *label)
        })),
        Target::Component(name) => match component_id(world, name) {
            Ok(Some(id)) => Matches::Nodes(app_nodes(world, |entity| entity.contains_id(id))),
            // A type never used as a component is on no entity.
            Ok(None) => Matches::Nodes(Vec::new()),
            Err(message) => Matches::NotANode(message),
        },
        Target::Within(outer, inner) => {
            let outers = match matches(world, outer) {
                Matches::Nodes(nodes) if nodes.is_empty() => {
                    return Matches::Missing(nothing_matches(world, outer));
                }
                Matches::Nodes(nodes) => nodes,
                other => return other,
            };
            match matches(world, inner) {
                Matches::Nodes(nodes) => Matches::Nodes(
                    nodes
                        .into_iter()
                        .filter(|node| outers.iter().any(|outer| is_below(world, *node, *outer)))
                        .collect(),
                ),
                other => other,
            }
        }
        Target::Nth(index, inner) => match matches(world, inner) {
            Matches::Nodes(nodes) => nodes.get(*index).map_or_else(
                || {
                    Matches::Missing(if nodes.is_empty() {
                        nothing_matches(world, inner)
                    } else {
                        format!(
                            "{target:?} counts from 0, but only {} match {inner:?}",
                            count_of(nodes.len())
                        )
                    })
                },
                |node| Matches::Nodes(vec![*node]),
            ),
            other => other,
        },
    }
}

fn count_of(count: usize) -> String {
    if count == 1 {
        "1 widget".to_owned()
    } else {
        format!("{count} widgets")
    }
}

fn app_nodes(world: &World, keep: impl Fn(&EntityRef<'_>) -> bool) -> Vec<Entity> {
    let mut found: Vec<Entity> = world.try_query::<EntityRef>().map_or_default(|query| {
        query
            .iter_manual(world)
            .filter(|entity| keep(entity) && !is_tooling(world, entity.id()))
            .map(|entity| entity.id())
            .collect()
    });
    in_document_order(world, &mut found);
    found
}

fn in_document_order(world: &World, entities: &mut [Entity]) {
    entities.sort_by_cached_key(|entity| tree_key(world, *entity));
}

fn tree_key(world: &World, entity: Entity) -> (Entity, Vec<usize>) {
    let mut indices = Vec::new();
    let mut current = entity;
    while let Some(parent) = world.get::<ChildOf>(current) {
        let parent = parent.parent();
        let index = world
            .get::<Children>(parent)
            .and_then(|children| children.iter().position(|child| *child == current))
            .unwrap_or(usize::MAX);
        indices.push(index);
        current = parent;
    }
    indices.reverse();
    (current, indices)
}

fn is_below(world: &World, entity: Entity, ancestor: Entity) -> bool {
    let mut current = entity;
    while let Some(parent) = world.get::<ChildOf>(current) {
        current = parent.parent();
        if current == ancestor {
            return true;
        }
    }
    false
}

fn nothing_matches(world: &World, target: &Target) -> String {
    match target {
        Target::Text(text) => {
            let shown = app_values(world, |entity| entity.get::<Text>().map(|t| t.0.clone()));
            format!(
                "no widget shows \"{text}\"{}",
                did_you_mean_quoted(text, &shown)
            )
        }
        Target::Name(name) => {
            let names = app_values(world, |entity| {
                entity.get::<Name>().map(|n| n.as_str().to_owned())
            });
            format!(
                "no entity is named \"{name}\"{}",
                did_you_mean_quoted(name, &names)
            )
        }
        Target::Role {
            role,
            label: Some(label),
        } => {
            let labels = app_values(world, |entity| {
                role.fits(entity)
                    .then(|| label_of(world, entity.id()))
                    .filter(|text| !text.is_empty())
            });
            format!(
                "no {role:?} is labelled \"{label}\"{}",
                did_you_mean_quoted(label, &labels)
            )
        }
        Target::Role { role, label: None } => format!("there is no {role:?}"),
        Target::Component(name) => format!("no entity has a `{name}`"),
        Target::Within(outer, inner) => match matches(world, inner) {
            Matches::Nodes(nodes) if !nodes.is_empty() => format!(
                "{inner:?} matches {}, but none below {outer:?}",
                count_of(nodes.len())
            ),
            _ => format!(
                "{}, below {outer:?} or anywhere",
                nothing_matches(world, inner)
            ),
        },
        Target::Screen(_) | Target::World(_) | Target::App(_) | Target::Nth(..) => {
            format!("nothing matches {target:?}")
        }
    }
}

fn app_values(world: &World, read: impl Fn(&EntityRef<'_>) -> Option<String>) -> Vec<String> {
    let mut values: Vec<String> = world.try_query::<EntityRef>().map_or_default(|query| {
        query
            .iter_manual(world)
            .filter(|entity| !is_tooling(world, entity.id()))
            .filter_map(|entity| read(&entity))
            .filter(|value| !value.is_empty())
            .collect()
    });
    values.sort_unstable();
    values.dedup();
    values
}

fn did_you_mean_quoted(wanted: &str, candidates: &[String]) -> String {
    let close = close_matches(wanted, candidates.iter().map(String::as_str));
    if close.is_empty() {
        return String::new();
    }
    let quoted: Vec<String> = close.iter().map(|value| format!("\"{value}\"")).collect();
    format!("; did you mean {}?", quoted.join(" or "))
}

fn ambiguous(world: &World, target: &Target, nodes: &[Entity]) -> String {
    let listed: Vec<String> = nodes
        .iter()
        .map(|node| describe_match(world, *node))
        .collect();
    format!(
        "{target:?} is ambiguous: it matches {}: {}; pick one with Nth(..) or narrow it with Within(..)",
        count_of(nodes.len()),
        listed.join(", ")
    )
}

fn describe_match(world: &World, entity: Entity) -> String {
    let what = describe_entity(world, entity);
    match (
        world.get::<ComputedNode>(entity),
        world.get::<UiGlobalTransform>(entity),
    ) {
        (Some(node), Some(transform)) => {
            let centre = transform.affine().translation * node.inverse_scale_factor;
            let centre = (centre * MESSAGE_PRECISION).round() / MESSAGE_PRECISION;
            format!("{what} ({entity}) at ({}, {})", centre.x, centre.y)
        }
        _ => format!("{what} ({entity})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::targets::WidgetRole;

    #[test]
    fn a_hint_quotes_the_close_values_and_is_empty_when_none_is_close() {
        let shown = ["Pause".to_owned(), "Settings".to_owned()];
        assert_eq!(
            did_you_mean_quoted("Pasue", &shown),
            "; did you mean \"Pause\"?"
        );
        assert_eq!(did_you_mean_quoted("Quit", &shown), "");
    }

    #[test]
    fn roles_are_tried_most_specific_first() {
        assert_eq!(WidgetRole::ALL.first(), Some(&WidgetRole::MenuButton));
        assert_eq!(WidgetRole::ALL.last(), Some(&WidgetRole::Button));
    }
}
