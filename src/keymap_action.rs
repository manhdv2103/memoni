use std::{borrow::Cow, fmt, mem, sync::LazyLock};

use anyhow::Result;
use egui::{Event, Key, Modifiers, PointerButton, RawInput};
use log::debug;

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum KeyOrPointerButton {
    Key(Key),
    PointerButton(PointerButton),
}
impl KeyOrPointerButton {
    pub fn name(&self) -> Cow<'static, str> {
        match self {
            KeyOrPointerButton::Key(key) => {
                if *key >= Key::A && *key <= Key::Z {
                    Cow::Owned(key.name().to_lowercase())
                } else {
                    Cow::Borrowed(key.symbol_or_name())
                }
            }
            KeyOrPointerButton::PointerButton(button) => Cow::Borrowed(match button {
                PointerButton::Primary => "<pointer-1>",
                PointerButton::Middle => "<pointer-2>",
                PointerButton::Secondary => "<pointer-3>",
                PointerButton::Extra1 => "<pointer-4>",
                PointerButton::Extra2 => "<pointer-5>",
            }),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct KeyChord {
    pub key: KeyOrPointerButton,
    pub mods: Modifiers,
}
impl KeyChord {
    pub fn of_key_chord(key: Key, mods: Modifiers) -> KeyChord {
        KeyChord {
            key: KeyOrPointerButton::Key(key),
            mods,
        }
    }

    pub fn of_key(key: Key) -> KeyChord {
        Self::of_key_chord(key, Modifiers::NONE)
    }

    pub fn of_ptr_btn_chord(btn: PointerButton, mods: Modifiers) -> KeyChord {
        KeyChord {
            key: KeyOrPointerButton::PointerButton(btn),
            mods,
        }
    }

    pub fn of_ptr_btn(btn: PointerButton) -> KeyChord {
        Self::of_ptr_btn_chord(btn, Modifiers::NONE)
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

        write_part(&self.key.name())?;

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

#[derive(Debug, Default, Copy, Clone)]
pub struct PasteModifier {
    pub trim: bool,
    pub and_enter: bool,
}

#[derive(Debug, Copy, Clone)]
pub enum Action {
    Key(KeyAction),
    Pointer(PointerAction),
}

#[derive(Debug, Copy, Clone)]
pub enum KeyAction {
    Paste(PasteModifier),
    QuickPaste(usize),
    Scroll(ScrollAction),
    Remove,
    Pin,
    HideWindow,
}

#[derive(Debug, Copy, Clone)]
pub enum PointerAction {
    Paste(PasteModifier),
}

#[rustfmt::skip]
static ACTION_KEYMAPS: LazyLock<Vec<(Vec<KeyChord>, Action)>> = LazyLock::new(|| {
    use Action::Key as AK;
    use Action::Pointer as AP;
    use KeyAction::*;
    use Key::*;
    use PointerButton::*;
    use KeyChord as KC;
    use Modifiers as M;
    vec![
        (vec![KC::of_key(ArrowUp)]                                , AK(Scroll(ScrollAction::ItemUp))),
        (vec![KC::of_key(ArrowDown)]                              , AK(Scroll(ScrollAction::ItemDown))),

        (vec![KC::of_key(K)]                                      , AK(Scroll(ScrollAction::ItemUp))),
        (vec![KC::of_key(J)]                                      , AK(Scroll(ScrollAction::ItemDown))),

        (vec![KC::of_key_chord(P, M::CTRL)]                       , AK(Scroll(ScrollAction::ItemUp))),
        (vec![KC::of_key_chord(N, M::CTRL)]                       , AK(Scroll(ScrollAction::ItemDown))),

        (vec![KC::of_key_chord(Tab, M::SHIFT)]                    , AK(Scroll(ScrollAction::ItemUp))),
        (vec![KC::of_key(Tab)]                                    , AK(Scroll(ScrollAction::ItemDown))),

        (vec![KC::of_key_chord(U, M::CTRL)]                       , AK(Scroll(ScrollAction::HalfUp))),
        (vec![KC::of_key_chord(D, M::CTRL)]                       , AK(Scroll(ScrollAction::HalfDown))),

        (vec![KC::of_key_chord(B, M::CTRL)]                       , AK(Scroll(ScrollAction::PageUp))),
        (vec![KC::of_key_chord(F, M::CTRL)]                       , AK(Scroll(ScrollAction::PageDown))),

        (vec![KC::of_key(G), KC::of_key(G)]                       , AK(Scroll(ScrollAction::ToTop))),
        (vec![KC::of_key_chord(G, M::SHIFT)]                      , AK(Scroll(ScrollAction::ToBottom))),

        (vec![KC::of_key(Enter)]                                  , AK(KeyAction::Paste(PasteModifier::default()))),
        (vec![KC::of_key(Space)]                                  , AK(KeyAction::Paste(PasteModifier::default()))),
        (vec![KC::of_ptr_btn(Primary)]                            , AP(PointerAction::Paste(PasteModifier::default()))),

        (vec![KC::of_key_chord(Enter, M::CTRL)]                   , AK(KeyAction::Paste(PasteModifier { and_enter: true, trim: false }))),
        (vec![KC::of_key_chord(Space, M::CTRL)]                   , AK(KeyAction::Paste(PasteModifier { and_enter: true, trim: false }))),
        (vec![KC::of_ptr_btn_chord(Primary, M::CTRL)]             , AP(PointerAction::Paste(PasteModifier { and_enter: true, trim: false }))),

        (vec![KC::of_key_chord(Enter, M::SHIFT)]                  , AK(KeyAction::Paste(PasteModifier { trim: true, and_enter: false }))),
        (vec![KC::of_key_chord(Space, M::SHIFT)]                  , AK(KeyAction::Paste(PasteModifier { trim: true, and_enter: false }))),
        (vec![KC::of_ptr_btn_chord(Primary, M::SHIFT)]            , AP(PointerAction::Paste(PasteModifier { trim: true, and_enter: false }))),

        (vec![KC::of_key_chord(Enter, M::SHIFT | M::CTRL)]        , AK(KeyAction::Paste(PasteModifier { trim: true, and_enter: true }))),
        (vec![KC::of_key_chord(Space, M::SHIFT | M::CTRL)]        , AK(KeyAction::Paste(PasteModifier { trim: true, and_enter: true }))),
        (vec![KC::of_ptr_btn_chord(Primary, M::SHIFT | M::CTRL)]  , AP(PointerAction::Paste(PasteModifier { trim: true, and_enter: true }))),

        (vec![KC::of_key(Num1)]                                   , AK(QuickPaste(0))),
        (vec![KC::of_key(Num2)]                                   , AK(QuickPaste(1))),
        (vec![KC::of_key(Num3)]                                   , AK(QuickPaste(2))),
        (vec![KC::of_key(Num4)]                                   , AK(QuickPaste(3))),
        (vec![KC::of_key(Num5)]                                   , AK(QuickPaste(4))),
        (vec![KC::of_key(Num6)]                                   , AK(QuickPaste(5))),
        (vec![KC::of_key(Num7)]                                   , AK(QuickPaste(6))),
        (vec![KC::of_key(Num8)]                                   , AK(QuickPaste(7))),
        (vec![KC::of_key(Num9)]                                   , AK(QuickPaste(8))),
        (vec![KC::of_key(Num0)]                                   , AK(QuickPaste(9))),

        (vec![KC::of_key(D), KC::of_key(D)]                       , AK(Remove)),
        (vec![KC::of_key(Delete)]                                 , AK(Remove)),

        (vec![KC::of_key(P)]                                      , AK(Pin)),

        (vec![KC::of_key(Escape)]                                 , AK(HideWindow)),
        (vec![KC::of_key(Q)]                                      , AK(HideWindow)),
    ]
});

pub struct KeymapAction {
    action_keymap_trie: Trie<&'static KeyChord, Action>,
    pub pending_keys: Vec<KeyChord>,
}
impl KeymapAction {
    pub fn new() -> Result<Self> {
        let mut action_keymap_trie = Trie::default();
        for (keymap, action) in ACTION_KEYMAPS.iter() {
            action_keymap_trie.insert(keymap, *action);
        }

        Ok(KeymapAction {
            action_keymap_trie,
            pending_keys: vec![],
        })
    }

    pub fn process_input(
        &mut self,
        egui_input: &mut RawInput,
    ) -> (Vec<KeyAction>, Vec<PointerAction>) {
        let mut key_actions = vec![];
        let mut pointer_actions = vec![];

        for event in mem::take(&mut egui_input.events) {
            let key_chord = match event {
                Event::Key {
                    key,
                    pressed,
                    modifiers,
                    ..
                } => {
                    if pressed {
                        Some(KeyChord::of_key_chord(key, modifiers))
                    } else {
                        None
                    }
                }
                event @ Event::PointerButton {
                    button,
                    pressed,
                    modifiers,
                    ..
                } => {
                    egui_input.events.push(event);
                    // pointer action activated on button release
                    if !pressed {
                        Some(KeyChord::of_ptr_btn_chord(button, modifiers))
                    } else {
                        None
                    }
                }
                event => {
                    egui_input.events.push(event);
                    None
                }
            };

            if let Some(key_chord) = key_chord {
                if key_chord.key == KeyOrPointerButton::Key(Key::Escape)
                    && !self.pending_keys.is_empty()
                {
                    debug!("received Escape, clearing pending keys");
                    self.pending_keys.clear();
                    continue;
                }

                self.pending_keys.push(key_chord);
                if let Some(keymap_node) = self.action_keymap_trie.get_node(&self.pending_keys) {
                    if let Some(action) = keymap_node.value {
                        debug!(
                            "converting keymap {:?} to action {action:?}",
                            self.pending_keys
                        );
                        match action {
                            Action::Key(key_action) => key_actions.push(key_action),
                            Action::Pointer(pointer_action) => pointer_actions.push(pointer_action),
                        }
                        self.pending_keys.clear();
                    } else {
                        debug!("continuing building keymap: {:?}", self.pending_keys);
                    }
                } else {
                    debug!("received invalid keymap: {:?}", self.pending_keys);
                    self.pending_keys.clear();
                }
            }
        }

        (key_actions, pointer_actions)
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
