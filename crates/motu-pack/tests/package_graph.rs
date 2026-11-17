//! End-to-end: OSM JSON -> graph -> `.mrdp` -> read back -> route.
//!
//! This is the path the phone will take: nothing is unpacked to disk, the graph
//! is loaded straight out of the package bytes.

use motu_core::LatLng;
use motu_pack::{PackReader, PackWriter, SourceInfo};
use motu_routing::RoadGraph;
use motu_routing::build_from_osm_json;

fn osm_extract() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "elements": [
            {"type": "node", "id": 1, "lat": 31.2300, "lon": 121.4700},
            {"type": "node", "id": 2, "lat": 31.2300, "lon": 121.4750,
             "tags": {"highway": "traffic_signals"}},
            {"type": "node", "id": 3, "lat": 31.2300, "lon": 121.4800},
            {"type": "way", "id": 10, "nodes": [1, 2, 3],
             "tags": {"highway": "residential", "surface": "asphalt", "lanes": "2"}}
        ]
    }))
    .expect("json")
}

#[test]
fn graph_survives_the_package_roundtrip_and_routes() {
    // Build the graph the same way the data-preparation step does.
    let (graph, _) = build_from_osm_json(&osm_extract()).expect("build");
    let packed_graph: Vec<u8>;
    {
        let dir = std::env::temp_dir().join("motu-pack-e2e");
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("graph.bin");
        graph.save(&path).expect("save");
        packed_graph = std::fs::read(&path).expect("read");
        std::fs::remove_file(&path).ok();
    }

    let mut writer = PackWriter::new();
    writer.add("graph.bin", packed_graph).expect("add");
    writer
        .add("traffic-prior.json", b"{\"source\":\"assumption\",\"buckets\":[]}".to_vec())
        .expect("add");

    let dir = std::env::temp_dir().join("motu-pack-e2e");
    std::fs::create_dir_all(&dir).expect("dir");
    let package = dir.join("e2e.mrdp");
    writer
        .write(&package, &SourceInfo::default())
        .expect("write package");

    let reader = PackReader::open(&package).expect("open package");
    reader.verify().expect("verify");

    let bytes = reader.entry("graph.bin").expect("graph entry");
    let restored = RoadGraph::from_bytes(bytes).expect("load graph from package");
    assert_eq!(restored.node_count(), graph.node_count());
    assert_eq!(restored.signals, graph.signals);
    assert_eq!(restored.ways[0].lanes, 2);

    let plan = restored
        .route(LatLng::new(31.2300, 121.4700), LatLng::new(31.2300, 121.4800))
        .expect("route from packaged graph");
    assert!(plan.distance_m > 800.0, "distance: {}", plan.distance_m);
    assert!(!plan.steps.is_empty(), "packaged graph must still produce steps");

    std::fs::remove_file(&package).ok();
}
