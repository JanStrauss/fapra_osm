use std::path::Path;
use std::sync::Arc;
use std::collections::BinaryHeap;
use std::cmp::Ordering;
use std::f64;
use iron::prelude::*;
use iron::status;
use staticfile::Static;
use mount::Mount;
use ordered_float::OrderedFloat;
use urlencoded::UrlEncodedQuery;
use rustc_serialize::json;
use time::PreciseTime;

#[derive(Debug, Clone)]
struct HeapEntry {
	node: usize,
	cost: f64,
}

#[derive(Debug, Clone, RustcEncodable, RustcDecodable)]
struct RoutingResult {
	duration: i64,
	route: Option<Route>
}

#[derive(Debug, Clone, RustcEncodable, RustcDecodable)]
struct Route {
	distance: f64,
	time: f64,
	path: Vec<[f64; 2]>
}

#[derive(Debug, Clone)]
struct PredecessorInfo {
	node: usize,
	edge: usize
}

impl Ord for HeapEntry {
	fn cmp(&self, other: &HeapEntry) -> Ordering {
		OrderedFloat(other.cost).cmp(&OrderedFloat(self.cost))
	}
}

impl PartialOrd for HeapEntry {
	fn partial_cmp(&self, other: &HeapEntry) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}

impl Eq for HeapEntry {
}

impl PartialEq for HeapEntry {
	fn eq(&self, other: &HeapEntry) -> bool {
		return (self.node == other.node) & &(OrderedFloat(other.cost).eq(&OrderedFloat(self.cost)))
	}
}


pub fn start(data: ::data::RoutingData) {
	let data_wrapped = Arc::new(data);
	let data_wrapped_2 = data_wrapped.clone();
	let data_wrapped_3 = data_wrapped.clone();

	let mut mount = Mount::new();

	mount.mount("/", Static::new(Path::new("web/")));
	mount.mount("/api/hello", move |r: &mut Request| get_hello(r, &data_wrapped));
	mount.mount("/api/graph", move |r: &mut Request| get_graph(r, &data_wrapped_2));
	mount.mount("/api/route", move |r: &mut Request| get_route(r, &data_wrapped_3));

	println!("server running on http://localhost:8080/");

	Iron::new(mount).http("127.0.0.1:8080").unwrap();
}

fn get_hello(req: &mut Request, data: &::data::RoutingData) -> IronResult<Response> {
	println!("Running get_hello handler, URL path: {:?}", req.url.path);
	Ok(Response::with((status::Ok, format!("HI! nodes: {}, edges: {}", data.internal_nodes.len(), data.internal_edges.len()))))
}

fn get_graph(req: &mut Request, data: &::data::RoutingData) -> IronResult<Response> {
	println!("Running get_graph handler, URL path: {:?}", req.url.path);
	Ok(Response::with((status::Ok, format!("nodes: {}, edges: {}", data.internal_nodes.len(), data.internal_edges.len()))))
}

fn get_route(req: &mut Request, data: &::data::RoutingData) -> IronResult<Response> {
	if let Ok(ref query_map) = req.get_ref::<UrlEncodedQuery> () {
		let source_raw = query_map.get("source").and_then(|list| list.first()).and_then(|string| Some(string.as_str())).unwrap_or("1133751511");
		let target_raw = query_map.get("target").and_then(|list| list.first()).and_then(|string| Some(string.as_str())).unwrap_or("27281797");
		let metric_raw = query_map.get("metric").and_then(|list| list.first()).and_then(|string| Some(string.as_str())).unwrap_or("time");
		let vehicle_raw = query_map.get("vehicle").and_then(|list| list.first()).and_then(|string| Some(string.as_str())).unwrap_or("car");

		let source = itry!(source_raw.parse::<i64>());
		let target = itry!(target_raw.parse::<i64>());

		let vehice = match vehicle_raw {
			"car" => ::data::FLAG_CAR,
			"bike" => ::data::FLAG_BIKE,
			"walk" => ::data::FLAG_WALK,
			_ => ::data::FLAG_CAR
		};

		let metric = match metric_raw {
			"time" => edge_cost_time,
			"distance" => edge_cost_distance,
			_ => edge_cost_distance
		};

		println!("doing routing from {} to {} for vehicle {} with metric {}", source, target, vehice, metric_raw);
		let start = PreciseTime::now();
		let result = run_dijkstra(&data, source, target, vehice, metric);
		let end = PreciseTime::now();
		//println!("route: {:?}", result);

		if let Some(route) = result {
			let result = RoutingResult { duration: start.to(end).num_milliseconds(), route: Some(route) };

			Ok(Response::with((status::Ok, json::encode(&result).unwrap())))
		} else {
			Ok(Response::with((status::NotFound)))
		}
	} else {
		Ok(Response::with((status::InternalServerError)))
	}
}

fn run_dijkstra<F>(data: &::data::RoutingData, source_osm: i64, target_osm: i64, constraints: u8, cost_func: F) -> Option<Route>
	where F: Fn(&::data::RoutingEdge, &f64) -> f64 {
	let vspeed = match constraints {
		::data::FLAG_CAR => 130.0 / 3.6,
		::data::FLAG_BIKE => 15.0 / 3.6,
		::data::FLAG_WALK => 5.0 / 3.6,
		_ => 130.0 / 3.6
	};

	let mut distance = vec![f64::INFINITY; data.internal_nodes.len()];
	let mut predecessor = vec![0; data.internal_nodes.len()];
	let mut predecessor_edge = vec![0; data.internal_nodes.len()];

	let source = data.osm_nodes.get(&source_osm).unwrap().internal_id;
	let target = data.osm_nodes.get(&target_osm).unwrap().internal_id;

	let mut heap = BinaryHeap::new();

	distance[source] = 0.0;
	heap.push(HeapEntry { node: source, cost: 0.0 });

	println!("begin dijkstra");

	while let Some(HeapEntry { node, cost }) = heap.pop() {
		if node == target {
			println!("found route");
			return build_route(source, target, &predecessor, &predecessor_edge, &data, &vspeed);
		}

		if cost > distance[node] { continue; }

		let (start, end) = offset_lookup(&node, &data);
		let edges = &data.internal_edges[start..end];

		for (i, edge) in edges.iter().enumerate() {
			if constraints & edge.constraints == 0 {
				continue;
			}
			let neighbor = HeapEntry { node: edge.target, cost: cost + cost_func(&edge, &vspeed) };

			if neighbor.cost < distance[neighbor.node] {
				distance[edge.target] = neighbor.cost;
				predecessor[edge.target] = node;
				predecessor_edge[edge.target] = i + start;
				heap.push(neighbor);
			}
		}
	}

	return None;
}

fn offset_lookup(node: &usize, data: &::data::RoutingData) -> (usize, usize) {
	let start = data.internal_offset[*node];
	let next_node = node + 1;
	let max_end = data.internal_offset[data.internal_offset.len() - 1];

	if next_node > data.internal_offset.len() - 1 {
		assert!(start <= max_end, "invalid offset lookup max!");

		return (start, max_end);
	}

	let end = data.internal_offset[next_node];

	assert!(start <= end, "invalid offset lookup!");

	return (start, end);
}


fn build_route(source: usize, target: usize, predecessor: &Vec<usize>, predecessor_edge: &Vec<usize>, data: &::data::RoutingData, vspeed: &f64) -> Option<Route> {
	let mut result = Route { distance: 0.0, time: 0.0, path: Vec::new() };

	let mut node = target;
	let mut edge = predecessor_edge[node];

	loop {
		if node == source {
			break;
		}

		let osm_id = data.internal_nodes[node];
		let pos = data.osm_nodes.get(&osm_id).unwrap().position;

		let mut speed = data.internal_edges[edge].speed;

		if *vspeed < speed {
			speed = *vspeed;
		}

		result.path.push([pos.lat, pos.lon]);
		result.distance += data.internal_edges[edge].length;
		result.time += data.internal_edges[edge].length / speed;

		node = predecessor[node];
		edge = predecessor_edge[node];
	}

	result.path.reverse();

	println!("build path");

	return Some(result);
}

fn edge_cost_distance(edge: &::data::RoutingEdge, vspeed: &f64) -> f64 {
	return edge.length;
}

fn edge_cost_time(edge: &::data::RoutingEdge, vspeed: &f64) -> f64 {
	let mut speed = edge.speed;

	if *vspeed < speed {
		speed = *vspeed;
	}

	return edge.length / speed;
}

#[test]
fn test_dijkstra() {
	let data = ::parser::build_dummy_data();

	let path = run_dijkstra(&data, 5000, 5003, ::data::FLAG_CAR, edge_cost_time);

	println!("path: {:?}", path);
}
