//! # cheap-ruler
//!
//! A collection of very fast approximations to common geodesic measurements.
//! Useful for performance-sensitive code that measures things on a city scale.
//!
//! This is a port of the cheap-ruler JS library and cheap-ruler-cpp C++ library
//! into safe Rust.
//!
//! Note: WGS84 ellipsoid is used instead of the Clarke 1866 parameters used by
//! the FCC formulas. See cheap-ruler-cpp#13 for more information.

#[macro_use]
extern crate geo_types;

use float_extras::f64::remainder;
use geo_types::{Coordinate, LineString, Point, Polygon};
use std::f64;
use std::iter;
use std::mem;

pub use distance_unit::DistanceUnit;
pub use point_on_line::PointOnLine;
pub use rect::Rect;

const RE: f64 = 6378.137; // equatorial radius in km
const FE: f64 = 1.0 / 298.257223563; // flattening
const E2: f64 = FE * (2.0 - FE);
const RAD: f64 = f64::consts::PI / 180.0;

/// A collection of very fast approximations to common geodesic measurements.
/// Useful for performance-sensitive code that measures things on a city scale.
/// Point coordinates are in the [x = longitude, y = latitude] form.
#[derive(Debug, PartialEq, Clone)]
pub struct CheapRuler {
    kx: f64,
    ky: f64,
    dkx: f64,
    dky: f64,
    distance_unit: DistanceUnit,
}

impl CheapRuler {
    pub fn new(latitude: f64, distance_unit: DistanceUnit) -> Self {
        // Curvature formulas from https://en.wikipedia.org/wiki/Earth_radius#Meridional
        let coslat = (latitude * RAD).cos();
        let w2 = 1.0 / (1.0 - E2 * (1.0 - coslat * coslat));
        let w = w2.sqrt();

        // multipliers for converting longitude and latitude degrees into distance
        let dkx = w * coslat; // based on normal radius of curvature
        let dky = w * w2 * (1.0 - E2); // based on meridonal radius of curvature

        let (kx, ky) = calculate_multipliers(distance_unit, dkx, dky);

        Self {
            kx,
            ky,
            dkx,
            dky,
            distance_unit,
        }
    }

    /// Creates a ruler object from tile coordinates (y and z). Convenient in
    /// tile-reduce scripts
    ///
    /// # Arguments
    ///
    /// * `y` - y
    /// * `z` - z
    /// * `distance_unit` - Unit to express distances in
    ///
    /// # Examples
    ///
    /// ```
    /// use cheap_ruler::{CheapRuler, DistanceUnit};
    /// let cr = CheapRuler::from_tile(1567, 12, DistanceUnit::Meters);
    /// ```
    pub fn from_tile(y: u32, z: u32, distance_unit: DistanceUnit) -> Self {
        assert!(z < 32);

        let n = f64::consts::PI
            * (1.0 - 2.0 * (y as f64 + 0.5) / ((1u32 << z) as f64));
        let latitude = n.sinh().atan() / RAD;

        Self::new(latitude, distance_unit)
    }

    /// Changes the ruler's unit to the given one
    ///
    /// # Arguments
    ///
    /// * `distance_unit` - New distance unit to express distances in
    pub fn change_unit(&mut self, distance_unit: DistanceUnit) {
        let (kx, ky) = calculate_multipliers(distance_unit, self.dkx, self.dky);
        self.distance_unit = distance_unit;
        self.kx = kx;
        self.ky = ky;
    }

    /// Clones the ruler to a new one with the given unit
    ///
    /// # Arguments
    ///
    /// * `distance_unit` - Distance unit to express distances in the new ruler
    pub fn clone_with_unit(&self, distance_unit: DistanceUnit) -> Self {
        let (kx, ky) = calculate_multipliers(distance_unit, self.dkx, self.dky);
        Self {
            distance_unit,
            kx,
            ky,
            dkx: self.dkx,
            dky: self.dky,
        }
    }

    /// Gets the distance unit that the ruler was instantiated with
    pub fn distance_unit(&self) -> DistanceUnit {
        self.distance_unit
    }

    /// Calculates the square of the approximate distance between two
    /// geographical points
    ///
    /// # Arguments
    ///
    /// * `a` - First point
    /// * `b` - Second point
    pub fn square_distance(&self, a: &Point<f64>, b: &Point<f64>) -> f64 {
        let dx = long_diff(a.lng(), b.lng()) * self.kx;
        let dy = (a.lat() - b.lat()) * self.ky;
        dx * dx + dy * dy
    }

    /// Calculates the approximate distance between two geographical points
    ///
    /// # Arguments
    ///
    /// * `a` - First point
    /// * `b` - Second point
    ///
    /// # Examples
    ///
    /// ```
    /// use cheap_ruler::{CheapRuler, DistanceUnit};
    /// let cr = CheapRuler::new(44.7192003, DistanceUnit::Meters);
    /// let dist = cr.distance(
    ///   &(14.8901816, 44.7209699).into(),
    ///   &(14.8905188, 44.7209699).into()
    /// );
    /// assert!(dist < 38.0);
    /// ```
    pub fn distance(&self, a: &Point<f64>, b: &Point<f64>) -> f64 {
        self.square_distance(a, b).sqrt()
    }

    /// Returns the bearing between two points in angles
    ///
    /// # Arguments
    ///
    /// * `a` - First point
    /// * `b` - Second point
    ///
    /// # Examples
    ///
    /// ```
    /// use cheap_ruler::{CheapRuler, DistanceUnit};
    /// let cr = CheapRuler::new(44.7192003, DistanceUnit::Meters);
    /// let bearing = cr.bearing(
    ///   &(14.8901816, 44.7209699).into(),
    ///   &(14.8905188, 44.7209699).into()
    /// );
    /// assert_eq!(bearing, 90.0);
    /// ```
    pub fn bearing(&self, a: &Point<f64>, b: &Point<f64>) -> f64 {
        let dx = long_diff(b.lng(), a.lng()) * self.kx;
        let dy = (b.lat() - a.lat()) * self.ky;

        dx.atan2(dy) / RAD
    }

    /// Returns a new point given distance and bearing from the starting point
    ///
    /// # Arguments
    ///
    /// * `origin` - origin point
    /// * `dist` - distance
    /// * `bearing` - bearing
    ///
    /// # Examples
    ///
    /// ```
    /// use cheap_ruler::{CheapRuler, DistanceUnit};
    /// let cr = CheapRuler::new(44.7192003, DistanceUnit::Meters);
    /// let p1 = (14.8901816, 44.7209699).into();
    /// let p2 = (14.8905188, 44.7209699).into();
    /// let dist = cr.distance(&p1, &p2);
    /// let bearing = cr.bearing(&p1, &p2);
    /// let destination = cr.destination(&p1, dist, bearing);
    ///
    /// assert_eq!(destination.lng(), p2.lng());
    /// assert_eq!(destination.lat(), p2.lat());
    /// ```
    pub fn destination(
        &self,
        origin: &Point<f64>,
        dist: f64,
        bearing: f64,
    ) -> Point<f64> {
        let a = bearing * RAD;
        self.offset(origin, a.sin() * dist, a.cos() * dist)
    }

    /// Returns a new point given easting and northing offsets (in ruler units)
    /// from the starting point
    ///
    /// # Arguments
    ///
    /// * `origin` - point
    /// * `dx` - easting
    /// * `dy` - northing
    pub fn offset(&self, origin: &Point<f64>, dx: f64, dy: f64) -> Point<f64> {
        (origin.lng() + dx / self.kx, origin.lat() + dy / self.ky).into()
    }

    /// Given a line (an array of points), returns the total line distance.
    ///
    /// # Arguments
    ///
    /// * `points` - line of points
    ///
    /// # Example
    ///
    /// ```
    /// use cheap_ruler::{CheapRuler, DistanceUnit};
    /// use geo_types::LineString;
    /// let cr = CheapRuler::new(50.458, DistanceUnit::Meters);
    /// let line_string: LineString<f64> = vec![
    ///     (-67.031, 50.458),
    ///     (-67.031, 50.534),
    ///     (-66.929, 50.534),
    ///     (-66.929, 50.458)
    /// ].into();
    /// let length = cr.line_distance(&line_string);
    /// ```
    pub fn line_distance(&self, points: &LineString<f64>) -> f64 {
        let line_iter = points.to_owned().into_iter();

        let left = iter::once(None).chain(line_iter.clone().map(Some));
        left.zip(line_iter)
            .map(|(a, b)| match a {
                Some(a) => self.distance(&a.into(), &b.into()),
                None => 0.0,
            })
            .sum()
    }

    /// Given a polygon returns the area
    ///
    /// * `polygon` - Polygon
    pub fn area(&self, polygon: &Polygon<f64>) -> f64 {
        // FIXME: subtract interiors
        let exterior = polygon
            .exterior()
            .points_iter()
            .collect::<Vec<Point<f64>>>();
        let mut sum = sum_area(&exterior);
        for interior in polygon.interiors() {
            let interior = interior.points_iter().collect::<Vec<Point<f64>>>();
            sum -= sum_area(&interior);
        }
        (sum.abs() / 2.0) * self.kx * self.ky
    }

    /// Returns the point at a specified distance along the line
    ///
    /// # Arguments
    ///
    /// * `line` - Line
    /// * `dist` - Distance along the line
    pub fn along(
        &self,
        line: &LineString<f64>,
        dist: f64,
    ) -> Option<Point<f64>> {
        let line_len = line.num_coords();
        if line_len == 0 {
            return None;
        }

        if dist <= 0.0 {
            return Some(line[0].into());
        }

        let last_index = line_len - 1;
        let mut sum = 0.0;
        for i in 0..last_index {
            let p0 = &line[i].into();
            let p1 = &line[i + 1].into();
            let d = self.distance(p0, p1);
            sum += d;
            if sum > dist {
                return Some(interpolate(p0, p1, (dist - (sum - d)) / d));
            }
        }
        Some(line[last_index].into())
    }

    /// Returns the shortest distance between a point and a line segment given
    /// with two points.
    ///
    /// # Arguments
    ///
    /// * `p` - Point to calculate the distance from
    /// * `start` - Start point of line segment
    /// * `end` - End point of line segment
    pub fn point_to_segment_distance(
        &self,
        p: &Point<f64>,
        start: &Point<f64>,
        end: &Point<f64>,
    ) -> f64 {
        let mut x = start.lng();
        let mut y = start.lat();
        let dx = long_diff(end.lng(), x) * self.kx;
        let dy = (end.lat() - y) * self.ky;

        if dx != 0.0 || dy != 0.0 {
            let t = (long_diff(p.lng(), x) * self.kx * dx
                + (p.lat() - y) * self.ky * dy)
                / (dx * dx + dy * dy);
            if t > 1.0 {
                x = end.lng();
                y = end.lat();
            } else if t > 0.0 {
                x += (dx / self.kx) * t;
                y += (dy / self.ky) * t;
            }
        }
        self.distance(&p, &point!(x: x, y: y))
    }

    /// Returns a tuple of the form (point, index, t) where point is closest
    /// point on the line from the given point, index is the start index of the
    /// segment with the closest point, and t is a parameter from 0 to 1 that
    /// indicates where the closest point is on that segment
    ///
    /// # Arguments
    ///
    /// * `line` - Line to compare with point
    /// * `point` - Point to calculate the closest point on the line
    pub fn point_on_line(
        &self,
        line: &LineString<f64>,
        point: &Point<f64>,
    ) -> Option<PointOnLine<f64>> {
        let mut min_dist = f64::INFINITY;
        let mut min_x = 0.0;
        let mut min_y = 0.0;
        let mut min_i = 0;
        let mut min_t = 0.0;

        let line_len = line.num_coords();
        if line_len == 0 {
            return None;
        }

        for i in 0..line_len - 1 {
            let mut t = 0.0;
            let mut x = line[i].x;
            let mut y = line[i].y;
            let dx = long_diff(line[i + 1].x, x) * self.kx;
            let dy = (line[i + 1].y - y) * self.ky;

            if dx != 0.0 || dy != 0.0 {
                t = (long_diff(point.lng(), x) * self.kx * dx
                    + (point.lat() - y) * self.ky * dy)
                    / (dx * dx + dy * dy);

                if t > 1.0 {
                    x = line[i + 1].x;
                    y = line[i + 1].y;
                } else if t > 0.0 {
                    x += (dx / self.kx) * t;
                    y += (dy / self.ky) * t;
                }
            }

            let d2 = self.square_distance(&point, &point!(x: x, y: y));

            if d2 < min_dist {
                min_dist = d2;
                min_x = x;
                min_y = y;
                min_i = i;
                min_t = t;
            }
        }

        Some(PointOnLine::new(
            point!(x: min_x, y: min_y),
            min_i,
            0f64.max(1f64.min(min_t)),
        ))
    }

    /// Returns a part of the given line between the start and the stop points
    /// (or their closest points on the line)
    ///
    /// # Arguments
    ///
    /// * `start` - Start point
    /// * `stop` - Stop point
    /// * `line` - Line string
    pub fn line_slice(
        &self,
        start: &Point<f64>,
        stop: &Point<f64>,
        line: &LineString<f64>,
    ) -> LineString<f64> {
        let pol1 = self.point_on_line(line, start);
        let pol2 = self.point_on_line(line, stop);

        if pol1.is_none() || pol2.is_none() {
            return line_string![];
        }
        let mut pol1 = pol1.unwrap();
        let mut pol2 = pol2.unwrap();

        if pol1.index() > pol2.index()
            || pol1.index() == pol2.index() && pol1.t() > pol2.t()
        {
            mem::swap(&mut pol1, &mut pol2);
        }

        let mut slice = vec![pol1.point()];

        let l = pol1.index() + 1;
        let r = pol2.index();

        if line[l] != slice[0].into() && l <= r {
            slice.push(line[l].into());
        }

        let mut i = l + 1;
        while i <= r {
            slice.push(line[i].into());
            i += 1;
        }

        if line[r] != pol2.point().into() {
            slice.push(pol2.point());
        }

        slice.into()
    }

    /// Returns a part of the given line between the start and the stop points
    /// indicated by distance along the line
    ///
    /// * `start` - Start distance
    /// * `stop` - Stop distance
    /// * `line` - Line string
    pub fn line_slice_along(
        &self,
        start: f64,
        stop: f64,
        line: &LineString<f64>,
    ) -> LineString<f64> {
        let mut sum = 0.0;
        let mut slice = vec![];

        if line.num_coords() == 0 {
            return slice.into();
        }

        for i in 0..line.num_coords() - 1 {
            let p0 = line[i].into();
            let p1 = line[i + 1].into();
            let d = self.distance(&p0, &p1);

            sum += d;

            if sum > start && slice.is_empty() {
                slice.push(interpolate(&p0, &p1, (start - (sum - d)) / d));
            }

            if sum >= stop {
                slice.push(interpolate(&p0, &p1, (stop - (sum - d)) / d));
                return slice.into();
            }

            if sum > start {
                slice.push(p1);
            }
        }

        slice.into()
    }

    /// Given a point, returns a bounding rectangle created from the given point
    /// buffered by a given distance
    ///
    /// # Arguments
    ///
    /// * `p` - Point
    /// * `buffer` - Buffer distance
    pub fn buffer_point(&self, p: &Point<f64>, buffer: f64) -> Rect<f64> {
        let v = buffer / self.ky;
        let h = buffer / self.kx;

        Rect::new(
            Coordinate {
                x: p.lng() - h,
                y: p.lat() - v,
            },
            Coordinate {
                x: p.lng() + h,
                y: p.lat() + v,
            },
        )
    }

    /// Given a bounding box, returns the box buffered by a given distance
    ///
    /// # Arguments
    ///
    /// * `bbox` - Bounding box
    /// * `buffer` - Buffer distance
    pub fn buffer_bbox(&self, bbox: &Rect<f64>, buffer: f64) -> Rect<f64> {
        let v = buffer / self.ky;
        let h = buffer / self.kx;

        Rect::new(
            Coordinate {
                x: bbox.min().x - h,
                y: bbox.min().y - v,
            },
            Coordinate {
                x: bbox.max().x + h,
                y: bbox.max().y + v,
            },
        )
    }

    /// Returns true if the given point is inside in the given bounding box,
    /// otherwise false.
    ///
    /// # Arguments
    ///
    /// * `p` - Point
    /// * `bbox` - Bounding box
    pub fn inside_bbox(&self, p: &Point<f64>, bbox: &Rect<f64>) -> bool {
        p.lat() >= bbox.min().y
            && p.lat() <= bbox.max().y
            && long_diff(p.lng(), bbox.min().x) >= 0.0
            && long_diff(p.lng(), bbox.max().x) <= 0.0
    }
}

pub fn interpolate(a: &Point<f64>, b: &Point<f64>, t: f64) -> Point<f64> {
    let dx = long_diff(b.lng(), a.lng());
    let dy = b.lat() - a.lat();
    Point::new(a.lng() + dx * t, a.lat() + dy * t)
}

fn calculate_multipliers(
    distance_unit: DistanceUnit,
    dkx: f64,
    dky: f64,
) -> (f64, f64) {
    let mul = distance_unit.conversion_factor_kilometers() * RAD * RE;
    let kx = mul * dkx;
    let ky = mul * dky;
    (kx, ky)
}

fn long_diff(a: f64, b: f64) -> f64 {
    remainder(a - b, 360.0)
}

fn sum_area(line: &[Point<f64>]) -> f64 {
    let line_len = line.len();
    let mut sum = 0.0;
    let mut k = line_len - 1;
    for j in 0..line_len {
        sum +=
            (line[j].lng() - line[k].lng()) * (line[j].lat() + line[k].lat());
        k = j;
    }
    sum
}

mod distance_unit;
mod point_on_line;
mod rect;
