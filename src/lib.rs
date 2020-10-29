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

#[cfg(test)]
extern crate assert_approx_eq;

use core::convert::From;
use float_extras::f64::remainder;
use geo_types::{Coordinate, LineString, Point, Polygon, Rect};
use num_traits::cast::NumCast;
use num_traits::Num;
use std::f64;
use std::iter;
use std::mem;

const RE: f64 = 6378.137; // equatorial radius in km
const FE: f64 = 1.0 / 298.257223563; // flattening
const E2: f64 = FE * (2.0 - FE);
const RAD: f64 = f64::consts::PI / 180.0;

/// Defines common units of distance that can be used
#[derive(Debug, PartialEq)]
pub enum DistanceUnit {
    Kilometers,
    Miles,
    NauticalMiles,
    Meters,
    Yards,
    Feet,
    Inches,
}

impl DistanceUnit {
    /// Provides a factor that scales the unit into kilometers
    fn conversion_factor_kilometers(&self) -> f64 {
        match *self {
            DistanceUnit::Kilometers => 1.0,
            DistanceUnit::Miles => 1000.0 / 1609.344,
            DistanceUnit::NauticalMiles => 1000.0 / 1852.0,
            DistanceUnit::Meters => 1000.0,
            DistanceUnit::Yards => 1000.0 / 0.9144,
            DistanceUnit::Feet => 1000.0 / 0.3048,
            DistanceUnit::Inches => 1000.0 / 0.0254,
        }
    }
}

pub struct PointOnLine<T>
where
    T: Num + NumCast + Copy + PartialEq + PartialOrd,
{
    point: Point<T>,
    index: usize,
    t: T,
}

impl<T> PointOnLine<T>
where
    T: Num + NumCast + Copy + PartialEq + PartialOrd,
{
    pub fn new(point: Point<T>, index: usize, t: T) -> Self {
        Self { point, index, t }
    }

    pub fn point(&self) -> Point<T> {
        self.point
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn t(&self) -> T {
        self.t
    }
}

impl<T> From<(Point<T>, usize, T)> for PointOnLine<T>
where
    T: Num + NumCast + Copy + PartialEq + PartialOrd,
{
    fn from(tuple: (Point<T>, usize, T)) -> PointOnLine<T> {
        PointOnLine::new(tuple.0, tuple.1, tuple.2)
    }
}

/// A collection of very fast approximations to common geodesic measurements.
/// Useful for performance-sensitive code that measures things on a city scale.
/// Point coordinates are in the [x = longitude, y = latitude] form.
#[derive(Debug, PartialEq, Clone)]
pub struct CheapRuler {
    kx: f64,
    ky: f64,
}

impl CheapRuler {
    pub fn new(latitude: f64, distance_unit: DistanceUnit) -> Self {
        // Curvature formulas from https://en.wikipedia.org/wiki/Earth_radius#Meridional
        let mul = distance_unit.conversion_factor_kilometers() * RAD * RE;
        let coslat = (latitude * RAD).cos();
        let w2 = 1.0 / (1.0 - E2 * (1.0 - coslat * coslat));
        let w = w2.sqrt();

        // multipliers for converting longitude and latitude degrees into distance
        let kx = mul * w * coslat; // based on normal radius of curvature
        let ky = mul * w * w2 * (1.0 - E2); // based on meridonal radius of curvature

        Self { kx, ky }
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

    /// Calculates the square of the approximate distance between two
    /// geographical points
    ///
    /// # Arguments
    ///
    /// * `a` - First point
    /// * `b` - Second point
    pub fn square_distance(&self, a: &Point<f64>, b: &Point<f64>) -> f64 {
        let dx = long_diff(a.x(), b.x()) * self.kx;
        let dy = (a.y() - b.y()) * self.ky;
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
        let dx = long_diff(b.x(), a.x()) * self.kx;
        let dy = (b.y() - a.y()) * self.ky;

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
    /// assert_eq!(destination.x(), p2.x());
    /// assert_eq!(destination.y(), p2.y());
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
        (origin.x() + dx / self.kx, origin.y() + dy / self.ky).into()
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

    /// Given a polygon (an array of rings, where each ring is an array of
    /// points), returns the area
    ///
    /// * `polygons` - Slice of polygons
    pub fn area(&self, polygons: &[Polygon<f64>]) -> f64 {
        let sum: f64 = polygons
            .iter()
            .enumerate()
            .map(|(i, ring)| {
                let ring = ring.exterior().to_owned().into_points();
                let mut j = 0;
                let mut k = ring.len() - 1;
                let mut sum = 0.0;
                while j < ring.len() {
                    sum += (ring[j].x() - ring[k].x())
                        * (ring[j].y() + ring[k].y())
                        * if i != 0 { -1.0 } else { 1.0 };
                    k = j;
                    j += 1;
                }
                sum
            })
            .sum();

        (sum.abs() / 2.0) * self.kx * self.ky
    }

    /// Returns the point at a specified distance along the line
    ///
    /// # Arguments
    ///
    /// * `line` - Line
    /// * `dist` - Distance along the line
    pub fn along(&self, line: &LineString<f64>, dist: f64) -> Point<f64> {
        if dist <= 0.0 {
            return line[0].into();
        }

        let last_index = line.num_coords() - 1;
        let mut sum = 0.0;
        for i in 0..last_index {
            let p0 = &line[i].into();
            let p1 = &line[i + 1].into();
            let d = self.distance(p0, p1);
            sum += d;
            if sum > dist {
                return interpolate(p0, p1, (dist - (sum - d)) / d);
            }
        }
        line[last_index].into()
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
    ) -> PointOnLine<f64> {
        let mut min_dist = f64::INFINITY;
        let mut min_x = 0.0;
        let mut min_y = 0.0;
        let mut min_i = 0;
        let mut min_t = 0.0;

        for i in 0..line.num_coords() - 1 {
            let mut t = 0.0;
            let mut x = line[i].x;
            let mut y = line[i].y;
            let dx = long_diff(line[i + 1].x, x) * self.kx;
            let dy = (line[i + 1].y - y) * self.ky;

            if dx != 0.0 || dy != 0.0 {
                t = (long_diff(point.x(), x) * self.kx * dx
                    + (point.y() - y) * self.ky * dy)
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

        ((min_x, min_y).into(), min_i, 0f64.max(1f64.min(min_t))).into()
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
        let mut pol1 = self.point_on_line(line, start);
        let mut pol2 = self.point_on_line(line, stop);

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
    /// * `buffer` - Buffer
    pub fn buffer_point(&self, p: Point<f64>, buffer: f64) -> Rect<f64> {
        let v = buffer / self.ky;
        let h = buffer / self.kx;

        Rect::new(
            Coordinate {
                x: p.x() - h,
                y: p.y() - v,
            },
            Coordinate {
                x: p.x() + h,
                y: p.y() + v,
            },
        )
    }

    /// Given a bounding box, returns the box buffered by a given distance
    ///
    /// # Arguments
    ///
    /// * `bbox` - Bounding box
    /// * `buffer` - Buffer
    pub fn buffer_bbox(&self, bbox: Rect<f64>, buffer: f64) -> Rect<f64> {
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
    pub fn inside_bbox(&self, p: Point<f64>, bbox: Rect<f64>) -> bool {
        p.y() >= bbox.min().y
            && p.y() <= bbox.max().y
            && long_diff(p.x(), bbox.min().x) >= 0.0
            && long_diff(p.x(), bbox.max().x) <= 0.0
    }
}

pub fn interpolate(a: &Point<f64>, b: &Point<f64>, t: f64) -> Point<f64> {
    let dx = long_diff(b.x(), a.x());
    let dy = b.y() - a.y();
    Point::new(a.x() + dx * t, a.y() + dy * t)
}

fn long_diff(a: f64, b: f64) -> f64 {
    remainder(a - b, 360.0)
}

/*
#[cfg(test)]
mod tests {
    use super::*;

    include!("../test-data/lines.rs");
    include!("../test-data/turf.rs");

    static CR_LATITUDE: f64 = 32.8351;

    fn cheap_ruler() -> CheapRuler {
        CheapRuler::new(CR_LATITUDE, DistanceUnit::Kilometers)
    }

    fn cheap_ruler_miles() -> CheapRuler {
        CheapRuler::new(CR_LATITUDE, DistanceUnit::Miles)
    }

    #[test]
    fn test_distance() {
        let ruler = cheap_ruler();

        for i in 0..POINTS.len() - 1 {
            let expected = TURF_DISTANCE[i];
            let p0 = POINTS[i].into();
            let p1 = POINTS[i + 1].into();
            let actual = ruler.distance(&p0, &p1);

            assert_approx_eq!(expected, actual, 0.003);
        }
    }

    #[test]
    fn test_distance_miles() {
        let ruler = cheap_ruler();
        let ruler_miles = cheap_ruler_miles();

        let p0 = (30.5, 32.8351).into();
        let p1 = (30.51, 32.8451).into();

        let d = ruler.distance(&p0, &p1);
        let d2 = ruler_miles.distance(&p0, &p1);

        assert_approx_eq!(d / d2, 1.609344, 1e-12);
    }

    #[test]
    fn test_bearing() {
        let ruler = cheap_ruler();

        for i in 0..POINTS.len() - 1 {
            let expected = TURF_BEARING[i];
            let p0 = POINTS[i].into();
            let p1 = POINTS[i + 1].into();
            let actual = ruler.bearing(&p0, &p1);

            assert_approx_eq!(expected, actual, 0.005);
        }
    }
}
*/
