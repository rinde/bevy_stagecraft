//! Reading state out of the world, for checks.

use bevy::{
    ecs::{
        component::Component,
        entity::Entity,
        hierarchy::{ChildOf, Children},
        name::Name,
        resource::Resource,
        world::World,
    },
    picking::{hover::HoverMap, pointer::PointerId},
    text::EditableText,
    ui::widget::Text,
};

/// A read-only view of the world handed to checks.
///
/// It offers a few generic helpers. An app adds its own by writing an
/// extension trait on `Probe` that goes through [`Probe::world`].
#[derive(Debug, Clone, Copy)]
pub struct Probe<'w> {
    /// The world being inspected.
    world: &'w World,
}

impl<'w> Probe<'w> {
    /// Wraps `world`.
    #[must_use]
    pub const fn new(world: &'w World) -> Self {
        Self { world }
    }

    /// The world, for helpers that need more than the generic ones.
    #[must_use]
    pub const fn world(&self) -> &'w World {
        self.world
    }

    /// How many entities have component `C`.
    #[must_use]
    pub fn count<C: Component>(&self) -> usize {
        // A component nobody has registered cannot be on any entity.
        self.world
            .try_query::<&C>()
            .map_or(0, |query| query.iter_manual(self.world).count())
    }

    /// The resource `R`, if it exists.
    #[must_use]
    pub fn resource<R: Resource>(&self) -> Option<&'w R> {
        self.world.get_resource::<R>()
    }

    /// The entities under the mouse pointer, nearest first.
    #[must_use]
    pub fn hovered(&self) -> Vec<Entity> {
        hovered_entities(self.world)
    }

    /// Whether some UI node shows exactly `text`: a text node, or a text input
    /// holding it.
    #[must_use]
    pub fn has_text(&self, text: &str) -> bool {
        let in_text = self
            .world
            .try_query::<(Entity, &Text)>()
            .is_some_and(|query| {
                query.iter_manual(self.world).any(|(entity, shown)| {
                    shown.0 == text && !crate::targets::is_tooling(self.world, entity)
                })
            });
        in_text
            || self
                .world
                .try_query::<(Entity, &EditableText)>()
                .is_some_and(|query| {
                    query.iter_manual(self.world).any(|(entity, field)| {
                        field.value() == text && !crate::targets::is_tooling(self.world, entity)
                    })
                })
    }
}

/// The entities the mouse pointer hovers, nearest first, ties by entity.
pub(crate) fn hovered_entities(world: &World) -> Vec<Entity> {
    let Some(map) = world.get_resource::<HoverMap>() else {
        return Vec::new();
    };
    let Some(hits) = map.get(&PointerId::Mouse) else {
        return Vec::new();
    };
    let mut hits: Vec<_> = hits
        .iter()
        .map(|(entity, hit)| (*entity, hit.depth))
        .collect();
    // Backends report depth as the distance from the camera, so smaller is
    // nearer. Ties break by entity so the order never depends on hashing.
    hits.sort_by(|(a, depth_a), (b, depth_b)| depth_a.total_cmp(depth_b).then(a.cmp(b)));
    hits.into_iter().map(|(entity, _)| entity).collect()
}

/// Fails unless `widget`, or something below it, is under the pointer.
/// `label` is how the script named the widget, for the message.
pub(crate) fn check_hover(world: &World, widget: Entity, label: &str) -> Result<(), String> {
    let hovered = hovered_entities(world);
    if hovered
        .iter()
        .any(|entity| is_within(world, *entity, widget))
    {
        return Ok(());
    }
    let wanted = describe_entity(world, widget);
    Err(hovered.first().map_or_else(
        || format!("{label} ({wanted}) is not under the pointer: nothing is"),
        |covering| {
            format!(
                "{label} ({wanted}) is not under the pointer: it is covered by {}",
                describe_entity(world, *covering)
            )
        },
    ))
}

/// A short human description of `entity`, for failure messages.
pub(crate) fn describe_entity(world: &World, entity: Entity) -> String {
    if let Some(name) = world.get::<Name>(entity) {
        return format!("`{name}`");
    }
    if let Some(text) = text_within(world, entity) {
        return format!("a node showing \"{text}\"");
    }
    format!("an unnamed node ({entity})")
}

/// The text of `entity` or of its first descendant that has any.
fn text_within(world: &World, entity: Entity) -> Option<String> {
    if let Some(text) = world.get::<Text>(entity) {
        return Some(text.0.clone());
    }
    let children = world.get::<Children>(entity)?;
    children.iter().find_map(|child| text_within(world, *child))
}

/// Whether `entity` is `ancestor` or lies below it.
pub(crate) fn is_within(world: &World, entity: Entity, ancestor: Entity) -> bool {
    let mut current = entity;
    loop {
        if current == ancestor {
            return true;
        }
        match world.get::<ChildOf>(current) {
            Some(parent) => current = parent.parent(),
            None => return false,
        }
    }
}
