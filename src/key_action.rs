use std::{fmt, mem, sync::LazyLock};

use anyhow::Result;
use egui::{Event, Key, Modifiers, RawInput};
use log::debug;

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct KeyChord {
    pub key: Key,
    pub mods: Modifiers,
}
impl KeyChord {
    pub fn of(key: Key, mods: Modifiers) -> KeyChord {
        KeyChord { key, mods }
    }

    pub fn of_key(key: Key) -> KeyChord {
        Self::of(key, Modifiers::NONE)
    }
}
impl fmt::Display for KeyChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        let mut write_part = |str| -> fmt::Result {
            if !first {
                write!(f, "-")?;
            }
            first = false;
            write!(f, "{str}")?;
            Ok(())
        };

        if self.mods.contains(Modifiers::CTRL) {
            write_part("C")?;
        }
        if self.mods.contains(Modifiers::ALT) {
            write_part("M")?;
        }
        if self.mods.contains(Modifiers::SHIFT) {
            write_part("S")?;
        }

        if self.key >= Key::A && self.key <= Key::Z {
            write_part(&self.key.name().to_lowercase())?;
        } else {
            write_part(self.key.symbol_or_name())?;
        }

        Ok(())
    }
}

#[derive(Debug, Copy, Clone)]
pub enum ScrollAction {
    ItemUp,
    ItemDown,
    HalfUp,
    HalfDown,
    PageUp,
    PageDown,
    ToTop,
    ToBottom,
}
impl ScrollAction {
    pub fn flipped(self) -> Self {
        use ScrollAction::*;
        match self {
            ItemUp => ItemDown,
            ItemDown => ItemUp,
            HalfUp => HalfDown,
            HalfDown => HalfUp,
            PageUp => PageDown,
            PageDown => PageUp,
            ToTop => ToBottom,
            ToBottom => ToTop,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum Action {
    Paste,
    Scroll(ScrollAction),
    Remove,
    HideWindow,
}

#[rustfmt::skip]
static ACTION_KEYMAPS: LazyLock<Vec<(Vec<KeyChord>, Action)>> = LazyLock::new(|| {
    use Action::*;
    use Key::*;
    use KeyChord as KC;
    use Modifiers as M;
    vec![
        (vec![KC::of_key(ArrowUp)]          , Scroll(ScrollAction::ItemUp)),
        (vec![KC::of_key(ArrowDown)]        , Scroll(ScrollAction::ItemDown)),

        (vec![KC::of_key(K)]                , Scroll(ScrollAction::ItemUp)),
        (vec![KC::of_key(J)]                , Scroll(ScrollAction::ItemDown)),

        (vec![KC::of(P, M::CTRL)]           , Scroll(ScrollAction::ItemUp)),
        (vec![KC::of(N, M::CTRL)]           , Scroll(ScrollAction::ItemDown)),

        (vec![KC::of(Tab, M::SHIFT)]        , Scroll(ScrollAction::ItemUp)),
        (vec![KC::of_key(Tab)]              , Scroll(ScrollAction::ItemDown)),

        (vec![KC::of(U, M::CTRL)]           , Scroll(ScrollAction::HalfUp)),
        (vec![KC::of(D, M::CTRL)]           , Scroll(ScrollAction::HalfDown)),

        (vec![KC::of(B, M::CTRL)]           , Scroll(ScrollAction::PageUp)),
        (vec![KC::of(F, M::CTRL)]           , Scroll(ScrollAction::PageDown)),

        (vec![KC::of_key(G), KC::of_key(G)] , Scroll(ScrollAction::ToTop)),
        (vec![KC::of(G, M::SHIFT)]          , Scroll(ScrollAction::ToBottom)),

        (vec![KC::of_key(Enter)]            , Action::Paste),
        (vec![KC::of_key(Space)]            , Action::Paste),

        (vec![KC::of_key(D), KC::of_key(D)] , Remove),
        (vec![KC::of_key(Delete)]           , Remove),

        (vec![KC::of_key(Escape)]           , HideWindow),
        (vec![KC::of_key(Q)]                , HideWindow),
    ]
});

pub struct KeyAction {
    action_keymap_trie: Trie<&'static KeyChord, Action>,
    pub pending_keys: Vec<KeyChord>,
}
impl KeyAction {
    pub fn new() -> Result<Self> {
        let mut action_keymap_trie = Trie::default();
        for (keymap, action) in ACTION_KEYMAPS.iter() {
            action_keymap_trie.insert(keymap, *action);
        }

        Ok(KeyAction {
            action_keymap_trie,
            pending_keys: vec![],
        })
    }

    pub fn process_input(&mut self, egui_input: &mut RawInput) -> Vec<Action> {
        let mut actions = vec![];
        for event in mem::take(&mut egui_input.events) {
            if let Event::Key {
                key,
                pressed,
                modifiers,
                ..
            } = event
            {
                if pressed {
                    if key == Key::Escape && !self.pending_keys.is_empty() {
                        debug!("received Escape, clearing pending keys");
                        self.pending_keys.clear();
                        continue;
                    }

                    self.pending_keys.push(KeyChord::of(key, modifiers));
                    if let Some(keymap_node) = self.action_keymap_trie.get_node(&self.pending_keys)
                    {
                        if let Some(action) = keymap_node.value {
                            debug!(
                                "converting keymap {:?} to action {action:?}",
                                self.pending_keys
                            );
                            actions.push(action);
                            self.pending_keys.clear();
                        } else {
                            debug!("continuing building keymap: {:?}", self.pending_keys);
                        }
                    } else {
                        debug!("received invalid keymap: {:?}", self.pending_keys);
                        self.pending_keys.clear();
                    }
                }
            } else {
                egui_input.events.push(event);
            }
        }

        actions
    }
}

// Extremely simple trie implementation

struct Trie<K, V> {
    value: Option<V>,
    next: Vec<(K, Trie<K, V>)>,
}

impl<K, V> Default for Trie<K, V> {
    fn default() -> Self {
        Trie {
            value: None,
            next: vec![],
        }
    }
}

impl<K, V> Trie<K, V>
where
    K: PartialEq + Clone,
{
    fn insert<I>(&mut self, keys: I, value: V)
    where
        I: IntoIterator<Item = K>,
    {
        let mut node = self;
        for key in keys {
            if let Some(pos) = node.next.iter().position(|(k, _)| *k == key) {
                node = &mut node.next[pos].1;
            } else {
                node.next.push((key.clone(), Trie::default()));
                let len = node.next.len();
                node = &mut node.next[len - 1].1;
            }
        }

        node.value = Some(value);
    }

    fn get_node<I>(&self, keys: I) -> Option<&Trie<K, V>>
    where
        I: IntoIterator<Item = K>,
    {
        let mut node = self;
        for key in keys {
            match node.next.iter().find(|(k, _)| *k == key) {
                Some((_, child)) => node = child,
                None => return None,
            }
        }

        Some(node)
    }
}
