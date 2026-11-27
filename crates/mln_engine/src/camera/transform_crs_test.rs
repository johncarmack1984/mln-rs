//! A transform in a registered projection: MapLibre Native's
//! `test/map/transform_crs.test.cpp`.

use std::sync::Arc;
use std::time::Duration;

use mln_style::crs::{Crs, CrsConverter, TileMatrixSet};
use mln_style::geo::{LatLng, LatLngBounds, WrapMode};
use mln_style::range::Range;
use mln_style::size::Size;
use mln_style::time::Instant;
use parking_lot::Mutex;

use super::Transform;
use crate::camera::{AnimationOptions, CameraLimits, CameraOptions, ConstrainMode, ViewportMode};
use crate::geo::ScreenCoordinate;
use crate::geo::world::WorldMapping;
use crate::render::terrain::ElevationRanges;
use crate::tests::crs::{self, identity, mapping};
use crate::tile::OverscaledTileID;
use crate::tile_store::tile_cover::{TileCoverParameters, mercator_covering_tiles, tile_cover};

fn ll(lat: f64, lng: f64) -> LatLng {
    LatLng::new(lat, lng).unwrap()
}

fn sc(x: f64, y: f64) -> ScreenCoordinate {
    ScreenCoordinate::new(x, y)
}

#[track_caller]
fn assert_near(expected: f64, actual: f64, tolerance: f64) {
    assert!((expected - actual).abs() <= tolerance, "{actual} is not within {tolerance} of {expected}");
}

/// A transform of `width` by `height` pixels in `world`.
fn transform_in(world: WorldMapping, width: u32, height: u32) -> Transform {
    let mut transform = Transform::new(ConstrainMode::HeightOnly, ViewportMode::Default);
    transform.resize(Size::new(width, height)).unwrap();
    transform.set_world(world);
    transform
}

fn camera(lat: f64, lng: f64, zoom: f64) -> CameraOptions {
    CameraOptions { center: Some(ll(lat, lng)), zoom: Some(zoom), ..CameraOptions::default() }
}

fn center(transform: &Transform) -> LatLng {
    transform.view().center(WrapMode::Unwrapped)
}

fn cover(transform: &Transform, z: u8) -> Vec<OverscaledTileID> {
    tile_cover(&TileCoverParameters::new(transform.view().clone()), z, Range::new(0, 22), None)
}

fn bounded(transform: &mut Transform, bounds: LatLngBounds) {
    let limits = CameraLimits { bounds, ..*transform.state().limits() };
    transform.set_limits(limits).unwrap();
}

// gtest: TransformInTheIdentityProjection.PlacesTheCenterInItsSquare
#[test]
fn the_identity_places_the_center_in_its_square() {
    let mut transform = transform_in(identity(), 512, 512);
    // Tile 1/1/0 of the identity spans lng 0..90 and lat 0..90, and fills
    // the view.
    transform.jump_to(&camera(45.0, 45.0, 1.0));

    assert_eq!(ll(45.0, 45.0), center(&transform));
    let pixel = transform.lat_lng_to_screen_coordinate(&ll(45.0, 45.0));
    assert_near(256.0, pixel.x, 1e-6);
    assert_near(256.0, pixel.y, 1e-6);
    let top_left = transform.screen_coordinate_to_lat_lng(&sc(0.0, 0.0), WrapMode::Unwrapped).unwrap();
    assert_near(90.0, top_left.latitude(), 1e-6);
    assert_near(0.0, top_left.longitude(), 1e-6);
    let bottom_right = transform.screen_coordinate_to_lat_lng(&sc(512.0, 512.0), WrapMode::Unwrapped).unwrap();
    assert_near(0.0, bottom_right.latitude(), 1e-6);
    assert_near(90.0, bottom_right.longitude(), 1e-6);
}

// gtest: TransformInTheIdentityProjection.StopsTheCenterAtTheEdgeOfItsSquare
#[test]
fn the_identity_stops_the_center_at_the_edge_of_its_square() {
    let mut transform = transform_in(identity(), 512, 512);
    transform.jump_to(&camera(0.0, 120.0, 2.0));

    assert_eq!(ll(0.0, 90.0), center(&transform));
}

// gtest: TransformInTheIdentityProjection.LeavesTheZoomToTheMinimumZoom
#[test]
fn the_identity_leaves_the_zoom_to_the_minimum_zoom() {
    let mut transform = transform_in(identity(), 1024, 1024);
    transform.jump_to(&camera(0.0, 0.0, 0.0));

    // The square is half the height of the view; Mercator would zoom in until
    // it filled it.
    assert_eq!(0.0, transform.view().zoom());
}

// gtest: TransformInTheIdentityProjection.KeepsALatitudePastTheMercatorLimit
#[test]
fn the_identity_keeps_a_latitude_past_the_mercator_limit() {
    let mut transform = transform_in(identity(), 512, 512);
    transform.jump_to(&camera(88.0, 10.0, 4.0));

    assert_eq!(ll(88.0, 10.0), center(&transform));
}

// gtest: TransformInTheIdentityProjection.KeepsTheCenterAcrossAProjectionChange
#[test]
fn the_center_stays_across_a_projection_change() {
    let mut transform = transform_in(WorldMapping::Mercator, 512, 512);
    transform.jump_to(&camera(30.0, 40.0, 3.0));
    transform.set_world(identity());

    assert_near(30.0, center(&transform).latitude(), 1e-9);
    assert_near(40.0, center(&transform).longitude(), 1e-9);

    transform.set_world(WorldMapping::Mercator);
    assert_near(30.0, center(&transform).latitude(), 1e-9);
    assert_near(40.0, center(&transform).longitude(), 1e-9);
}

// gtest: TransformInTheIdentityProjection.KeepsTheCenterInsideMaxBounds
#[test]
fn the_identity_keeps_the_center_inside_max_bounds() {
    let mut transform = transform_in(identity(), 512, 512);
    bounded(&mut transform, LatLngBounds::hull(ll(0.0, 0.0), ll(45.0, 45.0)));
    transform.jump_to(&camera(80.0, -60.0, 3.0));

    assert_near(45.0, center(&transform).latitude(), 1e-9);
    assert_near(0.0, center(&transform).longitude(), 1e-9);
}

#[test]
fn max_bounds_without_a_place_in_the_square_keep_the_center_anywhere_in_it() {
    // The identity, with no place for a location south of the equator.
    let north = crs::crs(
        "test:north",
        |[lng, lat]| if lat < 0.0 { [f64::NAN; 2] } else { [lng, lat] },
        |position| position,
        [-90.0, 90.0],
        180.0,
    );
    let mut transform = transform_in(mapping(north), 512, 512);
    bounded(&mut transform, LatLngBounds::hull(ll(-60.0, -30.0), ll(-30.0, 30.0)));
    transform.jump_to(&camera(45.0, 45.0, 2.0));

    assert!(transform.view().x().is_finite() && transform.view().y().is_finite());
    assert_near(45.0, center(&transform).latitude(), 1e-9);
    assert_near(45.0, center(&transform).longitude(), 1e-9);
}

/// The identity, which counts its batches behind a lock, so that it isn't
/// unwind safe.
struct CountingIdentity {
    batches: Arc<Mutex<u64>>,
}

impl CrsConverter for CountingIdentity {
    fn forward(&self, lng_lats: &[[f64; 2]], positions: &mut [[f64; 2]]) {
        *self.batches.lock() += 1;
        positions.copy_from_slice(lng_lats);
    }

    fn inverse(&self, positions: &[[f64; 2]], lng_lats: &mut [[f64; 2]]) {
        *self.batches.lock() += 1;
        lng_lats.copy_from_slice(positions);
    }
}

#[test]
fn a_converter_that_isnt_unwind_safe_places_the_center() {
    let batches = Arc::new(Mutex::new(0));
    let counting = Crs {
        name: "test:counting".to_owned(),
        converter: Box::new(CountingIdentity { batches: Arc::clone(&batches) }),
        tile_matrix_set: TileMatrixSet { origin: [-90.0, 90.0], extent_at_zoom0: 180.0 },
    };
    let mut transform = transform_in(mapping(counting), 512, 512);
    transform.jump_to(&camera(45.0, 45.0, 1.0));

    assert_near(45.0, center(&transform).latitude(), 1e-9);
    assert_near(45.0, center(&transform).longitude(), 1e-9);
    assert!(*batches.lock() > 0);
}

// gtest: TransformInTheIdentityProjection.RoundTripsTheFreeCamera
#[test]
fn the_identity_round_trips_the_free_camera() {
    let mut transform = transform_in(identity(), 512, 512);
    transform.jump_to(&CameraOptions { pitch: Some(40.0), bearing: Some(25.0), ..camera(20.0, -30.0, 5.0) });

    let free_camera = transform.view().free_camera_options();
    let mut round_trip = transform_in(identity(), 512, 512);
    round_trip.set_free_camera_options(&free_camera);

    assert_near(20.0, center(&round_trip).latitude(), 1e-9);
    assert_near(-30.0, center(&round_trip).longitude(), 1e-9);
    assert_near(5.0, round_trip.view().zoom(), 1e-9);
}

// gtest: TransformInTheIdentityProjection.CoversItsSquareWithoutWorldCopies
#[test]
fn the_identity_covers_its_square_without_world_copies() {
    let mut transform = transform_in(identity(), 1024, 1024);
    transform.jump_to(&camera(0.0, 0.0, 0.0));

    // The view is twice as wide as the square: Mercator would add the world
    // copies on both sides.
    assert_eq!(vec![OverscaledTileID::from_zxy(0, 0, 0)], cover(&transform, 0));
}

// gtest: TransformInTheIdentityProjection.CoversTheTileThatFillsTheView
#[test]
fn the_identity_covers_the_tile_that_fills_the_view() {
    let mut transform = transform_in(identity(), 512, 512);
    transform.jump_to(&camera(45.0, 45.0, 1.0));

    assert_eq!(vec![OverscaledTileID::from_zxy(1, 1, 0)], cover(&transform, 1));
}

// The cover over terrain, GL JS's `coveringTiles`, sorts its tiles by their
// distance from the center.
#[test]
fn the_cover_over_terrain_is_nearest_first_to_a_center_without_a_location_of_its_own() {
    let mut transform = transform_in(mapping(crs::disc("test:disc")), 512, 512);
    // 72 N, 72 W is the place 0.1, 0.1 of the square, outside the disc where
    // the inverse has answers: its location is the nearest answer, on the
    // disc's edge at 0.146, 0.146.
    transform.jump_to(&camera(72.0, -72.0, 3.0));

    // The view sees four tiles, the first the one under the center, as at a
    // center with a location of its own.
    let parameters = TileCoverParameters::new(transform.view().clone());
    let cover = mercator_covering_tiles(&parameters, 3, Range::new(0, 22), None, &ElevationRanges::default(), None);
    assert_eq!(4, cover.len(), "{cover:?}");
    assert_eq!(OverscaledTileID::from_zxy(3, 0, 0), cover[0]);
    assert_eq!(OverscaledTileID::from_zxy(3, 1, 1), cover[3]);
}

// gtest: TransformInARegisteredProjection.FliesPastTheAntimeridianWithoutWrapping
#[test]
fn a_flight_goes_past_the_antimeridian_without_wrapping() {
    let wide = crs::crs("test:wide", |position| position, |position| position, [-360.0, 360.0], 720.0);
    let mut transform = transform_in(mapping(wide), 512, 512);
    transform.jump_to(&camera(0.0, 170.0, 4.0));

    let start = Instant::now() + Duration::from_secs(3600);
    transform.set_frame_time(Some(start));
    let target = CameraOptions { center: Some(ll(0.0, 200.0)), ..CameraOptions::default() };
    transform.fly_to(&target, &AnimationOptions::new(Duration::from_secs(1)), false);
    transform.update_transitions(Some(start + Duration::from_millis(500)));
    // Halfway, eastward: a wrapping world would have taken the target as
    // -160 and gone the other way around.
    assert!(center(&transform).longitude() > 170.0, "{:?}", center(&transform));
    transform.update_transitions(Some(start + Duration::from_secs(1)));
    assert_near(200.0, center(&transform).longitude(), 1e-9);
}

// gtest: TransformInARegisteredProjection.KeepsACenterInsideTheMaxBoundsOfAPolarCap
#[test]
fn a_center_inside_the_max_bounds_of_a_polar_cap_stays() {
    let mut transform = transform_in(mapping(crs::north_polar_stereographic("test:polar-cap")), 512, 512);
    bounded(&mut transform, LatLngBounds::hull(ll(60.0, -180.0), ll(90.0, 180.0)));
    transform.jump_to(&camera(70.0, 45.0, 3.0));

    // The corners of the cap are the pole and a point on its edge; the cap's
    // world box keeps the center where it is.
    assert_near(70.0, center(&transform).latitude(), 1e-6);
    assert_near(45.0, center(&transform).longitude(), 1e-6);
}

// gtest: TransformInARegisteredProjection.CoversTheSameTilesWhenTheCenterCrossesTheAntimeridian
#[test]
fn the_cover_stays_as_the_center_crosses_the_antimeridian() {
    let mut transform = transform_in(mapping(crs::polar_stereographic("test:polar-antimeridian")), 512, 512);
    let mut cover_at = |lat: f64, lng: f64| {
        transform.jump_to(&camera(lat, lng, 2.0));
        let mut tiles = cover(&transform, 2);
        tiles.sort();
        tiles
    };

    // The world doesn't wrap, so 179.9 E and 179.9 W at 72 N are a few
    // hundred meters apart and see the same four tiles.
    let east = cover_at(72.0, 179.9);
    assert_eq!(4, east.len());
    assert_eq!(east, cover_at(72.0, -179.9));
}

// gtest: TransformInARegisteredProjection.KeepsTheViewInsideThePolarCapInScreenMode
#[test]
fn the_view_stays_inside_a_polar_cap_in_screen_mode() {
    let mut transform = transform_in(mapping(crs::north_polar_stereographic("test:polar-screen")), 512, 512);
    let cap = LatLngBounds::hull(ll(60.0, -180.0), ll(90.0, 180.0));
    bounded(&mut transform, cap);
    transform.set_constrain_mode(ConstrainMode::Screen);
    transform.ease_to(&camera(61.0, 0.0, 3.0), &AnimationOptions::default());

    // At 61 N the bottom of the view would reach past the cap's world box, so
    // the center moves toward the pole until the bottom of the view is on the
    // box's edge.
    assert!(center(&transform).latitude() > 61.0, "{:?}", center(&transform));
    assert_near(0.0, center(&transform).longitude(), 1e-6);
    let world = transform.view().world();
    let world_box = world.lng_lat_box_to_world_box(&cap);
    let bottom = transform.screen_coordinate_to_lat_lng(&sc(256.0, 512.0), WrapMode::Unwrapped).unwrap();
    assert_near(world_box.max.y, world.world_from_lat_lng(&bottom).y, 1e-6);
}

#[test]
fn a_registered_projection_stays_flat_in_a_flipped_viewport() {
    let mut transform = transform_in(identity(), 512, 512);
    transform.jump_to(&camera(45.0, 45.0, 1.0));

    transform.set_viewport_mode(ViewportMode::FlippedY);
    assert_eq!("identity", transform.view().world().name());
    assert!(!transform.view().is_globe());
    assert_near(45.0, center(&transform).latitude(), 1e-9);
    assert_near(45.0, center(&transform).longitude(), 1e-9);

    transform.set_viewport_mode(ViewportMode::Default);
    assert_eq!("identity", transform.view().world().name());
    assert!(!transform.view().is_globe());
}
