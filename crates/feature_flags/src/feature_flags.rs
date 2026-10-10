// Makes the derive macro's reference to `::feature_flags::FeatureFlagValue`
// resolve when the macro is invoked inside this crate itself.
extern crate self as feature_flags;

mod store;

pub use store::*;

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{App, Context, Global, Window};

impl Global for FeatureFlagStore {}

pub trait FeatureFlagValue:
    Sized + Clone + Eq + Default + std::fmt::Debug + Send + Sync + 'static
{
    /// Every possible value for this flag, in the order the UI should display them.
    fn all_variants() -> &'static [Self];

    /// A stable identifier for this variant used when persisting overrides.
    fn override_key(&self) -> &'static str;

    fn from_wire(wire: &str) -> Option<Self>;

    /// Human-readable label for use in the configuration UI.
    fn label(&self) -> &'static str {
        self.override_key()
    }

    /// The variant that represents "on" — what the store resolves to when
    /// staff rules, `enabled_for_all`, or a server announcement apply.
    ///
    /// For enum flags this is usually the same as [`Default::default`] (the
    /// variant marked `#[default]` in the derive). [`PresenceFlag`] overrides
    /// this so that `default() == Off` (the "unconfigured" state) but
    /// `on_variant() == On` (the "enabled" state).
    fn on_variant() -> Self {
        Self::default()
    }
}

/// Default value type for simple on/off feature flags.
///
/// The fallback value is [`PresenceFlag::Off`] so that an absent / unknown
/// flag reads as disabled; the `on_variant` override pins the "enabled"
/// state to [`PresenceFlag::On`] so staff / server / `enabled_for_all`
/// resolution still lights the flag up.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum PresenceFlag {
    On,
    #[default]
    Off,
}

/// Presence flags deref to a `bool` so call sites can use `if *flag` without
/// spelling out the enum variant — or pass them anywhere a `&bool` is wanted.
impl std::ops::Deref for PresenceFlag {
    type Target = bool;

    fn deref(&self) -> &bool {
        match self {
            PresenceFlag::On => &true,
            PresenceFlag::Off => &false,
        }
    }
}

impl FeatureFlagValue for PresenceFlag {
    fn all_variants() -> &'static [Self] {
        &[PresenceFlag::On, PresenceFlag::Off]
    }

    fn override_key(&self) -> &'static str {
        match self {
            PresenceFlag::On => "on",
            PresenceFlag::Off => "off",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            PresenceFlag::On => "On",
            PresenceFlag::Off => "Off",
        }
    }

    fn from_wire(_: &str) -> Option<Self> {
        Some(PresenceFlag::On)
    }

    fn on_variant() -> Self {
        PresenceFlag::On
    }
}

pub trait FeatureFlag {
    const NAME: &'static str;
    type Value: FeatureFlagValue;
    fn enabled_for_all() -> bool {
        false
    }
    fn watch<V: 'static>(cx: &mut Context<V>) {
        cx.observe_global::<FeatureFlagStore>(|_, cx| cx.notify())
            .detach();
    }
}

pub trait FeatureFlagViewExt<V: 'static> {
    fn when_flag_enabled<T: FeatureFlag>(
        &mut self,
        window: &mut Window,
        callback: impl Fn(&mut V, &mut Window, &mut Context<V>) + Send + Sync + 'static,
    );
}

impl<V> FeatureFlagViewExt<V> for Context<'_, V>
where
    V: 'static,
{
    fn when_flag_enabled<T: FeatureFlag>(
        &mut self,
        window: &mut Window,
        callback: impl Fn(&mut V, &mut Window, &mut Context<V>) + Send + Sync + 'static,
    ) {
        if self
            .try_global::<FeatureFlagStore>()
            .is_some_and(|f| f.has_flag::<T>(self))
        {
            self.defer_in(window, move |view, window, cx| {
                callback(view, window, cx);
            });
            return;
        }
        let subscription = Rc::new(RefCell::new(None));
        let inner = self.observe_global_in::<FeatureFlagStore>(window, {
            let subscription = subscription.clone();
            move |v, window, cx| {
                let has_flag = cx.global::<FeatureFlagStore>().has_flag::<T>(cx);
                if has_flag {
                    callback(v, window, cx);
                    subscription.take();
                }
            }
        });
        subscription.borrow_mut().replace(inner);
    }
}

pub trait FeatureFlagAppExt {
    fn has_flag<T: FeatureFlag>(&self) -> bool;
    fn flag_value<T: FeatureFlag>(&self) -> T::Value;
}

impl FeatureFlagAppExt for App {
    fn has_flag<T: FeatureFlag>(&self) -> bool {
        self.try_global::<FeatureFlagStore>()
            .map(|store| store.has_flag::<T>(self))
            .unwrap_or_else(|| FeatureFlagStore::has_flag_default::<T>())
    }
    fn flag_value<T: FeatureFlag>(&self) -> T::Value {
        self.try_global::<FeatureFlagStore>()
            .and_then(|store| store.try_flag_value::<T>(self))
            .unwrap_or_default()
    }
}
