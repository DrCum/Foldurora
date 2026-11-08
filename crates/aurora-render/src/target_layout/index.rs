//! Specific preflighted AVL physical-ID index; stable source positions.
//! No per-node allocation/deletion, shifts, hidden maps or recursion growth.

use super::Error;
use super::descriptor::Bounded;
use super::inventory::MAX_DEVICE_RECORDS;
use super::ownership::AllocationId;

// AVL height for at most 4096 nodes is below 24; also a hard recursion/probe cap.
const MAX_HEIGHT: u8 = 24;
#[derive(Clone, Copy)]
struct Node {
    id: AllocationId,
    position: usize,
    child: [Option<usize>; 2],
    height: u8,
}
pub(super) struct IdIndex {
    nodes: Bounded<Node>,
    root: Option<usize>,
    capacity: usize,
}
impl IdIndex {
    pub fn element_size() -> usize {
        std::mem::size_of::<Node>()
    }
    pub fn new(capacity: usize, bytes: u64) -> Result<Self, Error> {
        Ok(Self {
            nodes: Bounded::new(capacity, MAX_DEVICE_RECORDS, bytes)?,
            root: None,
            capacity,
        })
    }
    pub fn storage(&self) -> u64 {
        self.nodes.storage()
    }
    fn node(&self, i: usize) -> Result<Node, Error> {
        self.nodes.slice().get(i).copied().ok_or(Error::Invalid)
    }
    fn set(&mut self, i: usize, node: Node) -> Result<(), Error> {
        *self.nodes.items_mut().get_mut(i).ok_or(Error::Invalid)? = node;
        Ok(())
    }
    pub fn position(&self, id: AllocationId) -> Option<usize> {
        self.probe(id).0
    }
    pub fn probe(&self, id: AllocationId) -> (Option<usize>, usize) {
        let mut next = self.root;
        for steps in 1..=usize::from(MAX_HEIGHT) {
            let Some(i) = next else {
                return (None, steps - 1);
            };
            let Ok(n) = self.node(i) else {
                return (None, steps);
            };
            match id.cmp(&n.id) {
                std::cmp::Ordering::Equal => return (Some(n.position), steps),
                std::cmp::Ordering::Less => next = n.child[0],
                std::cmp::Ordering::Greater => next = n.child[1],
            }
        }
        (None, usize::from(MAX_HEIGHT))
    }
    fn height(&self, i: Option<usize>) -> Result<i16, Error> {
        i.map_or(Ok(0), |i| Ok(i16::from(self.node(i)?.height)))
    }
    fn update(&mut self, i: usize) -> Result<(), Error> {
        let mut n = self.node(i)?;
        let height = 1 + self.height(n.child[0])?.max(self.height(n.child[1])?);
        n.height = u8::try_from(height).map_err(|_| Error::Overflow)?;
        if n.height > MAX_HEIGHT {
            return Err(Error::Capacity);
        }
        self.set(i, n)
    }
    fn rotate(&mut self, i: usize, direction: usize) -> Result<usize, Error> {
        if direction > 1 {
            return Err(Error::Invalid);
        }
        let mut n = self.node(i)?;
        let top = n.child[1 - direction].ok_or(Error::Invalid)?;
        if top == i {
            return Err(Error::Invalid);
        }
        let mut t = self.node(top)?;
        n.child[1 - direction] = t.child[direction];
        t.child[direction] = Some(i);
        self.set(i, n)?;
        self.set(top, t)?;
        self.update(i)?;
        self.update(top)?;
        Ok(top)
    }
    fn put(&mut self, root: Option<usize>, id: AllocationId, position: usize, depth: u8) -> Result<usize, Error> {
        if depth >= MAX_HEIGHT {
            return Err(Error::Capacity);
        }
        let Some(i) = root else {
            let i = self.nodes.slice().len();
            self.nodes.push(Node {
                id,
                position,
                child: [None; 2],
                height: 1,
            })?;
            return Ok(i);
        };
        let mut n = self.node(i)?;
        let direction = match id.cmp(&n.id) {
            std::cmp::Ordering::Equal => return Err(Error::Invalid),
            std::cmp::Ordering::Less => 0,
            std::cmp::Ordering::Greater => 1,
        };
        n.child[direction] = Some(self.put(n.child[direction], id, position, depth.checked_add(1).ok_or(Error::Overflow)?)?);
        self.set(i, n)?;
        self.update(i)?;
        let balance = self.height(n.child[0])? - self.height(n.child[1])?;
        if balance > 1 {
            let left = n.child[0].ok_or(Error::Invalid)?;
            if id > self.node(left)?.id {
                n.child[0] = Some(self.rotate(left, 0)?);
                self.set(i, n)?;
            }
            self.rotate(i, 1)
        } else if balance < -1 {
            let right = n.child[1].ok_or(Error::Invalid)?;
            if id < self.node(right)?.id {
                n.child[1] = Some(self.rotate(right, 1)?);
                self.set(i, n)?;
            }
            self.rotate(i, 0)
        } else {
            Ok(i)
        }
    }
    pub fn insert(&mut self, id: AllocationId, position: usize) -> Result<(), Error> {
        if self.position(id).is_some() {
            return Err(Error::Invalid);
        }
        if position >= self.capacity {
            return Err(Error::Capacity);
        }
        self.root = Some(self.put(self.root, id, position, 0)?);
        Ok(())
    }
}
