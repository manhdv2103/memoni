use anyhow::Result;
use x11rb::{
    protocol::xproto::{Atom, ConnectionExt},
    xcb_ffi::XCBConnection,
};

pub struct AtomPool<'a> {
    conn: &'a XCBConnection,
    atom_prefix: &'a str,
    atoms: Vec<Atom>,
    counter: u8,
}

impl<'a> AtomPool<'a> {
    pub fn new(conn: &'a XCBConnection, atom_prefix: &'a str) -> Result<Self> {
        let mut atom_pool = AtomPool {
            conn,
            atom_prefix,
            atoms: vec![],
            counter: 0,
        };

        let mut initial_atoms = vec![
            atom_pool.create_atom()?,
            atom_pool.create_atom()?,
            atom_pool.create_atom()?,
            atom_pool.create_atom()?,
        ];
        atom_pool.atoms.append(&mut initial_atoms);

        Ok(atom_pool)
    }

    pub fn get(&mut self) -> Result<Atom> {
        match self.atoms.pop() {
            Some(a) => Ok(a),
            None => self.create_atom(),
        }
    }

    pub fn release(&mut self, atom: Atom) {
        self.atoms.push(atom);
    }

    fn create_atom(&mut self) -> Result<Atom> {
        let counter_str = self.counter.to_string();
        let mut name = Vec::with_capacity(self.atom_prefix.len() + counter_str.len());
        name.extend_from_slice(self.atom_prefix.as_bytes());
        name.extend_from_slice(counter_str.as_bytes());

        let atom = self.conn.intern_atom(false, &name)?.reply()?.atom;
        self.counter += 1;

        Ok(atom)
    }
}
