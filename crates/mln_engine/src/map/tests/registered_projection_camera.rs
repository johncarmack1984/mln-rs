//! A map in a registered projection, as its views, conversions and camera
//! helpers see it.

use mln_style::error::ErrorKind;
use mln_style::geo::{LatLng, WrapMode};
use mln_style::size::Size;

use crate::camera::{CameraOptions, MapProjection};
use crate::exec::queue::Queue;
use crate::geo::ScreenCoordinate;
use crate::geo::world::WorldMapping;
use crate::map::{Map, MapOptions};
use crate::storage::stub_file_source::{ResponseType, StubFileSource};
use crate::tests::{crs, registry_builder};

/// A map of `size` pixels square whose registry has the built-in projections
/// and `extra`.
fn map(size: u32, extra: Vec<mln_style::crs::Crs>, run_loop: &Queue<()>) -> Map {
    let mut builder = registry_builder();
    for crs in extra {
        builder.add_projection(crs).unwrap();
    }
    Map::with_file_source(
        StubFileSource::on_queue(run_loop, ResponseType::Asynchronous),
        MapOptions { size: Size::new(size, size), ..Default::default() },
        builder.build(),
        &crate::map::tests::pool(),
    )
    .expect("map")
}

/// A style without sources in the projection `projection`, centered on
/// `[lng, lat]` at `zoom`.
fn style(projection: &str, [lng, lat]: [f64; 2], zoom: f64) -> String {
    format!(
        r#"{{"version":8,"projection":{{"type":"{projection}"}},"center":[{lng},{lat}],"zoom":{zoom},"sources":{{}},"layers":[]}}"#
    )
}

fn ll(lat: f64, lng: f64) -> LatLng {
    LatLng::new(lat, lng).unwrap()
}

#[track_caller]
fn assert_near(expected: f64, actual: f64, tolerance: f64) {
    assert!((expected - actual).abs() <= tolerance, "{actual} is not within {tolerance} of {expected}");
}

#[test]
fn a_projection_from_a_view_converts_in_the_maps_registered_projection() {
    let run_loop = Queue::<()>::new();
    let mut map = map(512, vec![crs::north_polar_stereographic("test:polar")], &run_loop);
    map.load_style_json(&style("test:polar", [45.0, 70.0], 3.0));
    map.jump_to(&CameraOptions { bearing: Some(20.0), pitch: Some(30.0), ..CameraOptions::default() });

    let view = map.snapshot().view;
    let world = map.transform_state().view().world().clone();
    let projection = MapProjection::from_view(&view, &world).unwrap();
    let from_map = MapProjection::new(&map);
    for lat_lng in [ll(70.0, 45.0), ll(75.0, 10.0), ll(66.0, 80.0)] {
        let expected = from_map.pixel_for_lat_lng(&lat_lng);
        let actual = projection.pixel_for_lat_lng(&lat_lng);
        assert!((actual.x - expected.x).abs() < 1e-6 && (actual.y - expected.y).abs() < 1e-6, "{lat_lng:?}");
        let back = projection.lat_lng_for_pixel(&actual, WrapMode::Unwrapped).unwrap();
        assert_near(lat_lng.latitude(), back.latitude(), 1e-9);
        assert_near(lat_lng.longitude(), back.longitude(), 1e-9);
    }
    assert_eq!(map.snapshot().view, view.to_view(&world).unwrap().map_view(view.extent.scale_factor));
}

#[test]
fn a_view_of_the_identity_places_its_center_in_the_identity_square() {
    let run_loop = Queue::<()>::new();
    let mut map = map(512, Vec::new(), &run_loop);
    map.load_style_json(&style("identity", [45.0, 45.0], 1.0));

    // Tile 1/1/0 of the identity spans lng 0..90 and lat 0..90, and fills
    // the view; Mercator would put 45° N well above the view's middle.
    let projection = MapProjection::from_view(&map.snapshot().view, map.transform_state().view().world()).unwrap();
    let middle = projection.pixel_for_lat_lng(&ll(45.0, 45.0));
    assert_near(256.0, middle.x, 1e-9);
    assert_near(256.0, middle.y, 1e-9);
    let top_left = projection.lat_lng_for_pixel(&ScreenCoordinate::new(0.0, 0.0), WrapMode::Unwrapped).unwrap();
    assert_near(90.0, top_left.latitude(), 1e-6);
    assert_near(0.0, top_left.longitude(), 1e-6);
}

#[test]
fn a_view_whose_center_the_projection_has_no_place_for_is_refused() {
    let northern = crs::crs(
        "test:northern",
        |[lng, lat]| if lat < 0.0 { [f64::NAN; 2] } else { [lng, lat] },
        |position| position,
        [-90.0, 90.0],
        180.0,
    );
    let run_loop = Queue::<()>::new();
    let map = map(256, Vec::new(), &run_loop);
    let mut view = map.snapshot().view;
    view.camera.center = ll(-10.0, 0.0);

    let error = MapProjection::from_view(&view, &crs::mapping(northern)).err().map(|error| error.kind);
    assert_eq!(Some(ErrorKind::InvalidArgument), error);
}

/// A feature of tile `z/x/y` of type `type_`, whose rings are `rings` in
/// tile coordinates.
struct TileFeature {
    type_: mln_style::feature::FeatureType,
    rings: mln_style::feature::GeometryCollection,
}

impl mln_style::feature::GeometryTileFeature for TileFeature {
    fn get_type(&self) -> mln_style::feature::FeatureType {
        self.type_
    }

    fn get_value(&self, _key: &str) -> Option<mln_style::feature::Value> {
        None
    }

    fn get_geometries(
        &self,
    ) -> Result<std::borrow::Cow<'_, mln_style::feature::GeometryCollection>, mln_style::error::Error> {
        Ok(std::borrow::Cow::Borrowed(&self.rings))
    }
}

fn tile_feature(type_: mln_style::feature::FeatureType, rings: &[&[(i16, i16)]]) -> TileFeature {
    let rings = rings.iter().map(|ring| ring.iter().map(|&(x, y)| geometry::Point::new(x, y)).collect::<Vec<_>>());
    TileFeature { type_, rings: mln_style::feature::GeometryCollection::from(rings.collect::<Vec<_>>()) }
}

/// GL JS's "query returns geometry through the transform projection"
/// (`src/data/feature_index.test.ts`): the top left of tile 2/1/2 is a
/// quarter of the way across the identity's square and halfway down, at
/// 45° W on the equator; Web Mercator puts it at 90° W.
#[test]
fn a_queried_feature_has_the_location_the_identity_gives_its_tile_coordinates() {
    use mln_style::feature::{FeatureType, GeometryTileFeature};

    let tile = mln_style::tile_id::CanonicalTileID::new(2, 1, 2);
    let point = tile_feature(FeatureType::Point, &[&[(0, 0)]]);
    let converted =
        crate::tile::convert_feature(&point, &point.get_geometries().unwrap(), &tile, &crs::identity()).unwrap();
    assert_eq!(Some(geo_types::Geometry::Point(geo_types::Point::new(-45.0, 0.0))), converted.geometry);
    let mercator =
        crate::tile::convert_feature(&point, &point.get_geometries().unwrap(), &tile, &WorldMapping::Mercator).unwrap();
    let Some(geo_types::Geometry::Point(mercator)) = mercator.geometry else { panic!("a point: {mercator:?}") };
    assert_eq!(-90.0, mercator.x());
    assert_near(0.0, mercator.y(), 1e-12);

    // A square with a square hole, its rings in order, each corner where the
    // identity puts it: a tile's 8192 units are 45 degrees at zoom 2.
    let square = &[(0, 0), (8192, 0), (8192, 8192), (0, 8192), (0, 0)][..];
    let hole = &[(2048, 2048), (2048, 6144), (6144, 6144), (6144, 2048), (2048, 2048)][..];
    let polygon = tile_feature(FeatureType::Polygon, &[square, hole]);
    let converted =
        crate::tile::convert_feature(&polygon, &polygon.get_geometries().unwrap(), &tile, &crs::identity()).unwrap();
    let Some(geo_types::Geometry::Polygon(converted)) = converted.geometry else {
        panic!("a polygon: {converted:?}");
    };
    let lng_lats =
        |ring: &geo_types::LineString<f64>| ring.0.iter().map(|coord| (coord.x, coord.y)).collect::<Vec<_>>();
    assert_eq!(
        vec![(-45.0, 0.0), (0.0, 0.0), (0.0, -45.0), (-45.0, -45.0), (-45.0, 0.0)],
        lng_lats(converted.exterior())
    );
    assert_eq!(
        vec![(-33.75, -11.25), (-33.75, -33.75), (-11.25, -33.75), (-11.25, -11.25), (-33.75, -11.25)],
        lng_lats(&converted.interiors()[0])
    );
}
