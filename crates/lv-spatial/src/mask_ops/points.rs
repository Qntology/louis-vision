//! Interactive seed clicks serialized in `[frame, object_id, positive, x, y]` order.

use serde::{Deserialize, Serialize};

type PointTuple = (u32, u32, u8, i32, i32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataPoint {
    pub frame: u32,
    pub object_id: u32,
    pub positive: bool,
    pub x: i32,
    pub y: i32,
}

impl From<PointTuple> for DataPoint {
    fn from((frame, object_id, positive, x, y): PointTuple) -> Self {
        Self { frame, object_id, positive: positive != 0, x, y }
    }
}

impl From<DataPoint> for PointTuple {
    fn from(p: DataPoint) -> Self {
        (p.frame, p.object_id, u8::from(p.positive), p.x, p.y)
    }
}

impl Serialize for DataPoint {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        PointTuple::from(*self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for DataPoint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        PointTuple::deserialize(deserializer).map(DataPoint::from)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Prompts {
    pub coords: Vec<[f32; 2]>,
    pub labels: Vec<i32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PointStore {
    points: Vec<DataPoint>,
}

impl PointStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn points(&self) -> &[DataPoint] {
        &self.points
    }

    pub fn add(&mut self, point: DataPoint) {
        self.points.push(point);
    }

    pub fn remove_last(&mut self) -> Option<DataPoint> {
        self.points.pop()
    }

    pub fn clear_all(&mut self) {
        self.points.clear();
    }

    pub fn clear_frame(&mut self, frame: u32) -> Vec<DataPoint> {
        let (removed, kept): (Vec<_>, Vec<_>) = self.points.drain(..).partition(|p| p.frame == frame);
        self.points = kept;
        removed
    }

    pub fn clear_object(&mut self, object_id: u32) -> Vec<DataPoint> {
        let (removed, kept): (Vec<_>, Vec<_>) = self.points.drain(..).partition(|p| p.object_id == object_id);
        self.points = kept;
        removed
    }

    pub fn prompts(&self, frame: u32, object_id: u32) -> Prompts {
        let mut prompts = Prompts::default();
        for p in self.points.iter().filter(|p| p.frame == frame && p.object_id == object_id) {
            prompts.coords.push([p.x as f32, p.y as f32]);
            prompts.labels.push(i32::from(p.positive));
        }
        prompts
    }

    pub fn objects(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.points.iter().map(|p| p.object_id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}
