use std::any::TypeId;

use gpui::{App, BorrowAppContext, Subscription};
use settings::SettingsStore;

use crate::{FeatureFlag, FeatureFlagValue};

pub struct FeatureFlagDescriptor {
    pub name: &'static str,
    pub variants: fn() -> Vec<FeatureFlagVariant>,
    pub on_variant_key: fn() -> &'static str,
    pub default_variant_key: fn() -> &'static str,
    pub enabled_for_all: fn() -> bool,
    pub type_id: fn() -> TypeId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureFlagVariant {
    pub override_key: &'static str,
    pub label: &'static str,
}

inventory::collect!(FeatureFlagDescriptor);

#[doc(hidden)]
pub mod __private {
    pub use inventory;
}

/// Submits a [`FeatureFlagDescriptor`] for this flag so it shows up in the
/// configuration UI and in `FeatureFlagStore::known_flags()`.
#[macro_export]
macro_rules! register_feature_flag {
    ($flag:ty) => {
        $crate::__private::inventory::submit! {
            $crate::FeatureFlagDescriptor {
                name: <$flag as $crate::FeatureFlag>::NAME,
                variants: || {
                    <<$flag as $crate::FeatureFlag>::Value as $crate::FeatureFlagValue>::all_variants()
                        .iter()
                        .map(|v| $crate::FeatureFlagVariant {
                            override_key: <<$flag as $crate::FeatureFlag>::Value as $crate::FeatureFlagValue>::override_key(v),
                            label: <<$flag as $crate::FeatureFlag>::Value as $crate::FeatureFlagValue>::label(v),
                        })
                        .collect()
                },
                on_variant_key: || {
                    <<$flag as $crate::FeatureFlag>::Value as $crate::FeatureFlagValue>::override_key(
                        &<<$flag as $crate::FeatureFlag>::Value as $crate::FeatureFlagValue>::on_variant(),
                    )
                },
                default_variant_key: || {
                    <<$flag as $crate::FeatureFlag>::Value as $crate::FeatureFlagValue>::override_key(
                        &<<$flag as $crate::FeatureFlag>::Value as ::std::default::Default>::default(),
                    )
                },
                enabled_for_all: <$flag as $crate::FeatureFlag>::enabled_for_all,
                type_id: || std::any::TypeId::of::<$flag>(),
            }
        }
    };
}

#[derive(Default)]
pub struct FeatureFlagStore {
    _settings_subscription: Option<Subscription>,
}

impl FeatureFlagStore {
    pub fn init(cx: &mut App) {
        let subscription = cx.observe_global::<SettingsStore>(|cx| {
            cx.update_default_global::<FeatureFlagStore, _>(|_, _| {});
        });

        cx.update_default_global::<FeatureFlagStore, _>(|store, _| {
            store._settings_subscription = Some(subscription);
        });
    }

    pub fn known_flags() -> impl Iterator<Item = &'static FeatureFlagDescriptor> {
        let mut seen = collections::HashSet::default();
        inventory::iter::<FeatureFlagDescriptor>().filter(move |d| seen.insert((d.type_id)()))
    }

    pub fn try_flag_value<T: FeatureFlag>(&self, _cx: &App) -> Option<T::Value> {
        if T::enabled_for_all() {
            return Some(T::Value::on_variant());
        }
        None
    }

    /// Whether the flag resolves to its "on" value. Best for presence-style
    /// flags. For enum flags with meaningful non-default variants, prefer
    /// [`crate::FeatureFlagAppExt::flag_value`].
    pub fn has_flag<T: FeatureFlag>(&self, cx: &App) -> bool {
        self.try_flag_value::<T>(cx)
            .is_some_and(|v| v == T::Value::on_variant())
    }

    /// Mirrors the resolution order of [`Self::try_flag_value`], but falls
    /// back to the [`Default`] variant when no rule applies so the UI always
    /// shows *something* selected — matching what
    /// [`crate::FeatureFlagAppExt::flag_value`] would return.
    pub fn resolved_key(&self, descriptor: &FeatureFlagDescriptor, _cx: &App) -> &'static str {
        if (descriptor.enabled_for_all)() {
            return (descriptor.on_variant_key)();
        }
        (descriptor.default_variant_key)()
    }

    pub fn is_forced_on(descriptor: &FeatureFlagDescriptor) -> bool {
        (descriptor.enabled_for_all)()
    }

    pub fn has_flag_default<T: FeatureFlag>() -> bool {
        T::enabled_for_all()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FeatureFlag, PresenceFlag};

    struct DemoFlag;
    impl FeatureFlag for DemoFlag {
        const NAME: &'static str = "demo";
        type Value = PresenceFlag;
    }

    struct AlwaysOnFlag;
    impl FeatureFlag for AlwaysOnFlag {
        const NAME: &'static str = "always-on";
        type Value = PresenceFlag;
        fn enabled_for_all() -> bool {
            true
        }
    }

    #[gpui::test]
    fn absent_flag_defaults_to_off(cx: &mut App) {
        let store = FeatureFlagStore::default();
        assert_eq!(store.try_flag_value::<DemoFlag>(cx), None);
        assert!(!store.has_flag::<DemoFlag>(cx));
        assert_eq!(PresenceFlag::default(), PresenceFlag::Off);
    }

    #[gpui::test]
    fn enabled_for_all_resolves_on(cx: &mut App) {
        let store = FeatureFlagStore::default();
        assert_eq!(
            store.try_flag_value::<AlwaysOnFlag>(cx),
            Some(PresenceFlag::On)
        );
        assert!(store.has_flag::<AlwaysOnFlag>(cx));
        assert!(FeatureFlagStore::has_flag_default::<AlwaysOnFlag>());
        assert!(!FeatureFlagStore::has_flag_default::<DemoFlag>());
    }
}
