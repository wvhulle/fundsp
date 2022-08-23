//! Network of AudioUnits connected together.

use super::audionode::*;
use super::audiounit::*;
use super::buffer::*;
use super::callback::*;
use super::math::*;
use super::signal::*;
use super::*;
use duplicate::duplicate_item;

pub type NodeIndex = usize;
pub type PortIndex = usize;

const ID: u64 = 63;

/// Input or output port.
#[derive(Clone, Copy)]
pub enum Port {
    /// Node input or output.
    Local(NodeIndex, PortIndex),
    /// Network input or output.
    Global(PortIndex),
    /// Unconnected input. Unconnected output ports are not marked anywhere.
    Zero,
}

#[derive(Clone, Copy)]
pub struct Edge {
    pub source: Port,
    pub target: Port,
}

/// Create an edge from source to target.
pub fn edge(source: Port, target: Port) -> Edge {
    Edge { source, target }
}

#[duplicate_item(
    f48       Vertex48       AudioUnit48;
    [ f64 ]   [ Vertex64 ]   [ AudioUnit64 ];
    [ f32 ]   [ Vertex32 ]   [ AudioUnit32 ];
)]
/// Individual AudioUnits are vertices in the graph.
pub struct Vertex48 {
    /// The unit.
    pub unit: Box<dyn AudioUnit48>,
    /// Edges connecting into this vertex. The length is equal to the number of inputs.
    pub source: Vec<Edge>,
    /// Input buffers. The length is equal to the number of inputs.
    pub input: Buffer<f48>,
    /// Output buffers. The length is equal to the number of outputs.
    pub output: Buffer<f48>,
    /// Input for tick iteration. The length is equal to the number of inputs.
    pub tick_input: Vec<f48>,
    /// Output for tick iteration. The length is equal to the number of outputs.
    pub tick_output: Vec<f48>,
    /// Index or ID of this unit. This equals unit index in graph.
    pub id: NodeIndex,
    /// This is set if all vertex inputs are sourced from matching outputs of the indicated node.
    pub source_vertex: Option<NodeIndex>,
}

#[duplicate_item(
    f48       Vertex48       AudioUnit48;
    [ f64 ]   [ Vertex64 ]   [ AudioUnit64 ];
    [ f32 ]   [ Vertex32 ]   [ AudioUnit32 ];
)]
impl Vertex48 {
    pub fn new(id: NodeIndex, inputs: usize, outputs: usize) -> Self {
        Self {
            unit: Box::new(super::prelude::pass()),
            source: vec![],
            input: Buffer::with_size(inputs),
            output: Buffer::with_size(outputs),
            tick_input: vec![0.0; inputs],
            tick_output: vec![0.0; outputs],
            id,
            source_vertex: None,
        }
    }

    pub fn inputs(&self) -> usize {
        self.input.buffers()
    }

    pub fn outputs(&self) -> usize {
        self.output.buffers()
    }

    pub fn update_source_vertex(&mut self) {
        self.source_vertex = None;
        if self.inputs() == 0 {
            return;
        }
        let mut source_node = 0;
        for i in 0..self.inputs() {
            match self.source[i].source {
                Port::Local(node, port) => {
                    if port != i {
                        return;
                    }
                    if i == 0 {
                        source_node = node;
                    } else if source_node != node {
                        return;
                    }
                }
                _ => {
                    return;
                }
            }
        }
        self.source_vertex = Some(source_node);
    }
}

#[duplicate_item(
    f48       Net48       Vertex48       AudioUnit48         Callback48;
    [ f64 ]   [ Net64 ]   [ Vertex64 ]   [ AudioUnit64 ]   [ Callback64 ];
    [ f32 ]   [ Net32 ]   [ Vertex32 ]   [ AudioUnit32 ]   [ Callback32 ];
)]
/// Network unit. It can contain other units and maintain connections between them.
/// Outputs of the network are sourced from user specified unit outputs or global inputs.
pub struct Net48 {
    /// Global input buffers.
    input: Buffer<f48>,
    /// Global output buffers.
    output: Buffer<f48>,
    /// Sources of global outputs.
    output_edge: Vec<Edge>,
    /// Vertices of the graph.
    vertex: Vec<Vertex48>,
    /// Ordering of vertex evaluation.
    order: Vec<NodeIndex>,
    ordered: bool,
    sample_rate: f64,
    tmp_vertex: Vertex48,
    callback: Option<Callback48<Net48>>,
}

#[duplicate_item(
    f48       Net48       Vertex48       AudioUnit48         Callback48;
    [ f64 ]   [ Net64 ]   [ Vertex64 ]   [ AudioUnit64 ]   [ Callback64 ];
    [ f32 ]   [ Net32 ]   [ Vertex32 ]   [ AudioUnit32 ]   [ Callback32 ];
)]
impl Net48 {
    /// Create a new network with the given number of inputs and outputs.
    /// The number of inputs and outputs is fixed after construction.
    pub fn new(inputs: usize, outputs: usize) -> Self {
        let mut net = Self {
            input: Buffer::with_size(inputs),
            output: Buffer::with_size(outputs),
            output_edge: vec![],
            vertex: vec![],
            order: vec![],
            ordered: true,
            sample_rate: DEFAULT_SR,
            tmp_vertex: Vertex48::new(0, 0, 0),
            callback: None,
        };
        for channel in 0..outputs {
            net.output_edge
                .push(edge(Port::Zero, Port::Global(channel)));
        }
        net
    }

    /// Set the update callback. The arguments to the callback are
    /// time in seconds, time since the last update in seconds,
    /// and the network itself.
    pub fn set_callback(
        &mut self,
        update_interval: f48,
        callback: Box<dyn FnMut(f48, f48, &mut Net48) + Send>,
    ) {
        self.callback = Some(Callback48::new(update_interval, callback));
    }

    /// Add a new unit to the network. Return its ID handle.
    /// ID handles are always consecutive numbers starting from zero.
    /// The unit is reset with the sample rate of the network.
    pub fn add(&mut self, mut unit: Box<dyn AudioUnit48>) -> NodeIndex {
        unit.reset(Some(self.sample_rate));
        let id = self.vertex.len();
        let inputs = unit.inputs();
        let outputs = unit.outputs();
        let mut vertex = Vertex48 {
            unit,
            source: vec![],
            input: Buffer::with_size(inputs),
            output: Buffer::with_size(outputs),
            tick_input: vec![0.0; inputs],
            tick_output: vec![0.0; outputs],
            id,
            source_vertex: None,
        };
        for i in 0..vertex.inputs() {
            vertex.source.push(edge(Port::Zero, Port::Local(id, i)));
        }
        self.vertex.push(vertex);
        // Note. We have designed the hash to depend on vertices but not edges.
        let hash = self.ping(true, AttoRand::new(ID));
        self.ping(false, hash);
        self.ordered = false;
        id
    }

    /// Replaces the given node in the network.
    /// The replacement must have the same number of inputs and outputs
    /// as the node it is replacing.
    pub fn replace(&mut self, node: NodeIndex, replacement: Box<dyn AudioUnit48>) {
        assert!(replacement.inputs() == self.node(node).inputs());
        assert!(replacement.outputs() == self.node(node).outputs());
        self.vertex[node].unit = replacement;
    }

    /// Connect the given unit output (`source`, `source_port`)
    /// to the given unit input (`target`, `target_port`).
    pub fn connect(
        &mut self,
        source: NodeIndex,
        source_port: PortIndex,
        target: NodeIndex,
        target_port: PortIndex,
    ) {
        self.vertex[target].source[target_port] = edge(
            Port::Local(source, source_port),
            Port::Local(target, target_port),
        );
        self.ordered = false;
    }

    /// Connect the node input (`target`, `target_port`)
    /// to the network input `global_input`.
    pub fn connect_input(
        &mut self,
        global_input: PortIndex,
        target: NodeIndex,
        target_port: PortIndex,
    ) {
        self.vertex[target].source[target_port] =
            edge(Port::Global(global_input), Port::Local(target, target_port));
        self.ordered = false;
    }

    /// Pipe global input to node `target`.
    /// Number of node inputs must match the number of network inputs.
    pub fn pipe_input(&mut self, target: NodeIndex) {
        assert!(self.vertex[target].inputs() == self.inputs());
        for i in 0..self.inputs() {
            self.vertex[target].source[i] = edge(Port::Global(i), Port::Local(target, i));
        }
        self.ordered = false;
    }

    /// Connect node output (`source`, `source_port`) to network output `global_output`.
    pub fn connect_output(
        &mut self,
        source: NodeIndex,
        source_port: PortIndex,
        global_output: PortIndex,
    ) {
        self.output_edge[global_output] = edge(
            Port::Local(source, source_port),
            Port::Global(global_output),
        );
        self.ordered = false;
    }

    /// Pipe node outputs to global outputs.
    /// Number of outputs must match the number of network outputs.
    pub fn pipe_output(&mut self, source: NodeIndex) {
        assert!(self.vertex[source].outputs() == self.outputs());
        for i in 0..self.outputs() {
            self.output_edge[i] = edge(Port::Local(source, i), Port::Global(i));
        }
        self.ordered = false;
    }

    /// Add an arbitrary edge to the network.
    pub fn join(&mut self, edge: Edge) {
        match edge.target {
            Port::Global(global_output) => self.output_edge[global_output] = edge,
            Port::Local(target, target_port) => self.vertex[target].source[target_port] = edge,
            _ => (),
        }
        self.ordered = false;
    }

    /// Connect `source` to `target`.
    /// The number of outputs in `source` and number of inputs in `target` must match.
    pub fn pipe(&mut self, source: NodeIndex, target: NodeIndex) {
        assert!(self.vertex[source].outputs() == self.vertex[target].inputs());
        for i in 0..self.vertex[target].inputs() {
            self.vertex[target].source[i] = edge(Port::Local(source, i), Port::Local(target, i));
        }
        self.ordered = false;
    }

    /// Assuming this network is a chain of processing units ordered by increasing node ID,
    /// add a new unit to the chain. Global outputs will be assigned to the outputs of the unit
    /// if possible. The number of inputs to the unit must match the number of outputs of the
    /// previous unit, or the number of network inputs if there is no previous unit.
    /// Returns the ID of the new unit.
    pub fn chain(&mut self, unit: Box<dyn AudioUnit48>) -> NodeIndex {
        let unit_inputs = unit.inputs();
        let unit_outputs = unit.outputs();
        let id = self.add(unit);
        if self.outputs() == unit_outputs {
            self.pipe_output(id);
        }
        if unit_inputs > 0 {
            if id > 0 {
                self.pipe(id - 1, id);
            } else {
                self.pipe_input(id);
            }
        }
        id
    }

    /// Access node.
    pub fn node(&self, node: NodeIndex) -> &dyn AudioUnit48 {
        &*self.vertex[node].unit
    }

    /// Access mutable node.
    pub fn node_mut(&mut self, node: NodeIndex) -> &mut dyn AudioUnit48 {
        &mut *self.vertex[node].unit
    }

    /// Indicate to callback handler that time is about to elapse.
    fn elapse(&mut self, dt: f48) {
        let mut tmp = self.callback.take();
        if let Some(cb) = &mut tmp {
            cb.update(dt, self);
        }
        self.callback = tmp.take();
    }

    /// Compute and store node order for this network.
    fn determine_order(&mut self) {
        self.ordered = true;
        for vertex in self.vertex.iter_mut() {
            vertex.update_source_vertex();
        }
        let mut order = Vec::new();
        self.determine_order_in(&mut order);
        self.order.clear();
        std::mem::swap(&mut order, &mut self.order);
    }

    /// Determine node order in the supplied vector.
    fn determine_order_in(&self, order: &mut Vec<NodeIndex>) {
        let mut vertices_left = self.vertex.len();
        let mut vertex_left = vec![true; self.vertex.len()];
        // Note about contents of the edge vector.
        // Each node input appears there exactly once.
        // Sources, however, are not unique or guaranteed to appear.
        let mut all_edges: Vec<Edge> = Vec::new();
        for vertex in self.vertex.iter() {
            for edge in &vertex.source {
                all_edges.push(*edge);
            }
        }

        let mut inputs_left = vec![0; self.vertex.len()];
        for i in 0..inputs_left.len() {
            inputs_left[i] = self.vertex[i].unit.inputs();
            if inputs_left[i] == 0 {
                vertex_left[i] = false;
                order.push(i);
                vertices_left -= 1;
            }
        }

        // Start from network inputs.
        for (_, edge) in all_edges.iter().enumerate() {
            if let (Port::Global(_) | Port::Zero, Port::Local(vertex, _)) =
                (edge.source, edge.target)
            {
                if vertex_left[vertex] {
                    inputs_left[vertex] -= 1;
                    if inputs_left[vertex] == 0 {
                        vertex_left[vertex] = false;
                        order.push(vertex);
                        vertices_left -= 1;
                    }
                }
            }
        }
        while vertices_left > 0 {
            let mut progress = false;
            for (_i, edge) in all_edges.iter().enumerate() {
                if let (Port::Local(source, _), Port::Local(target, _)) = (edge.source, edge.target)
                {
                    if !vertex_left[source] && vertex_left[target] {
                        progress = true;
                        inputs_left[target] -= 1;
                        if inputs_left[target] == 0 {
                            vertex_left[target] = false;
                            order.push(target);
                            vertices_left -= 1;
                        }
                    }
                }
            }
            // TODO. Make this a recoverable error.
            if !progress {
                panic!("Cycle detected.");
            }
        }
    }
}

#[duplicate_item(
    f48       Net48       Vertex48       AudioUnit48         Callback48;
    [ f64 ]   [ Net64 ]   [ Vertex64 ]   [ AudioUnit64 ]   [ Callback64 ];
    [ f32 ]   [ Net32 ]   [ Vertex32 ]   [ AudioUnit32 ]   [ Callback32 ];
)]
impl AudioUnit48 for Net48 {
    fn inputs(&self) -> usize {
        self.input.buffers()
    }

    fn outputs(&self) -> usize {
        self.output.buffers()
    }

    fn reset(&mut self, sample_rate: Option<f64>) {
        if let Some(sr) = sample_rate {
            self.sample_rate = sr;
        }
        for vertex in &mut self.vertex {
            vertex.unit.reset(sample_rate);
        }
        // Take the opportunity to unload some calculations.
        if !self.ordered {
            self.determine_order();
        }

        if let Some(cb) = &mut self.callback {
            cb.reset();
        }
    }

    fn tick(&mut self, input: &[f48], output: &mut [f48]) {
        if !self.ordered {
            self.determine_order();
        }
        self.elapse(1.0 / self.sample_rate as f48);
        // Iterate units in network order.
        for node_index in self.order.iter() {
            std::mem::swap(&mut self.tmp_vertex, &mut self.vertex[*node_index]);
            for channel in 0..self.tmp_vertex.inputs() {
                match self.tmp_vertex.source[channel].source {
                    Port::Zero => self.tmp_vertex.tick_input[channel] = 0.0,
                    Port::Global(port) => self.tmp_vertex.tick_input[channel] = input[port],
                    Port::Local(source, port) => {
                        self.tmp_vertex.tick_input[channel] = self.vertex[source].tick_output[port]
                    }
                }
            }
            self.tmp_vertex.unit.tick(
                &self.tmp_vertex.tick_input,
                &mut self.tmp_vertex.tick_output,
            );
            std::mem::swap(&mut self.tmp_vertex, &mut self.vertex[*node_index]);
        }

        // Then we set the global outputs.
        for channel in 0..output.len() {
            match self.output_edge[channel].source {
                Port::Global(port) => output[channel] = input[port],
                Port::Local(node, port) => output[channel] = self.vertex[node].tick_output[port],
                Port::Zero => output[channel] = 0.0,
            }
        }
    }

    fn process(&mut self, size: usize, input: &[&[f48]], output: &mut [&mut [f48]]) {
        if !self.ordered {
            self.determine_order();
        }
        self.elapse(size as f48 / self.sample_rate as f48);
        // Iterate units in network order.
        for node_index in self.order.iter() {
            std::mem::swap(&mut self.tmp_vertex, &mut self.vertex[*node_index]);
            if let Some(source_node) = self.tmp_vertex.source_vertex {
                // We can source inputs directly from a source vertex.
                self.tmp_vertex.unit.process(
                    size,
                    self.vertex[source_node].output.self_ref(),
                    self.tmp_vertex.output.self_mut(),
                );
            } else {
                // Gather inputs for this vertex.
                for channel in 0..self.tmp_vertex.inputs() {
                    match self.tmp_vertex.source[channel].source {
                        Port::Zero => self.tmp_vertex.input.mut_at(channel)[..size].fill(0.0),
                        Port::Global(port) => self.tmp_vertex.input.mut_at(channel)[..size]
                            .copy_from_slice(&input[port][..size]),
                        Port::Local(source, port) => {
                            self.tmp_vertex.input.mut_at(channel)[..size]
                                .copy_from_slice(&self.vertex[source].output.at(port)[..size]);
                        }
                    }
                }
                self.tmp_vertex.unit.process(
                    size,
                    self.tmp_vertex.input.self_ref(),
                    self.tmp_vertex.output.self_mut(),
                );
            }
            std::mem::swap(&mut self.tmp_vertex, &mut self.vertex[*node_index]);
        }

        // Then we set the global outputs.
        for channel in 0..output.len() {
            match self.output_edge[channel].source {
                Port::Global(port) => output[channel][..size].copy_from_slice(&input[port][..size]),
                Port::Local(node, port) => output[channel][..size]
                    .copy_from_slice(&self.vertex[node].output.at(port)[..size]),
                Port::Zero => output[channel][..size].fill(0.0),
            }
        }
    }

    fn get_id(&self) -> u64 {
        ID
    }

    fn set_hash(&mut self, hash: u64) {
        let mut hash = AttoRand::new(hash);
        for x in self.vertex.iter_mut() {
            x.unit.set_hash(hash.get());
        }
    }
    fn ping(&mut self, probe: bool, hash: AttoRand) -> AttoRand {
        if !probe {
            self.set_hash(hash.value());
        }
        let mut hash = hash.hash(ID);
        for x in self.vertex.iter_mut() {
            hash = x.unit.ping(probe, hash);
        }
        hash
    }

    fn route(&self, input: &SignalFrame, frequency: f64) -> SignalFrame {
        let mut inner_signal: Vec<SignalFrame> = vec![];
        for vertex in self.vertex.iter() {
            inner_signal.push(new_signal_frame(vertex.unit.outputs()));
        }
        let mut tmp_order = vec![];
        for &unit_index in if self.ordered {
            self.order.iter()
        } else {
            self.determine_order_in(&mut tmp_order);
            tmp_order.iter()
        } {
            let mut input_signal = new_signal_frame(self.vertex[unit_index].unit.inputs());
            for channel in 0..self.vertex[unit_index].unit.inputs() {
                match self.vertex[unit_index].source[channel].source {
                    Port::Local(j, port) => input_signal[channel] = inner_signal[j][port],
                    Port::Global(j) => input_signal[channel] = input[j],
                    Port::Zero => input_signal[channel] = Signal::Value(0.0),
                }
            }
            inner_signal[unit_index] = self.vertex[unit_index].unit.route(&input_signal, frequency);
        }

        // Then we set the global outputs.
        let mut output_signal = new_signal_frame(self.outputs());
        for channel in 0..self.outputs() {
            match self.output_edge[channel].source {
                Port::Global(port) => output_signal[channel] = input[port],
                Port::Local(node, port) => {
                    output_signal[channel] = inner_signal[node][port];
                }
                Port::Zero => output_signal[channel] = Signal::Value(0.0),
            }
        }
        output_signal
    }

    fn set(&mut self, parameter: audionode::Tag, value: f64) {
        for vertex in &mut self.vertex {
            vertex.unit.set(parameter, value);
        }
    }

    fn get(&self, parameter: Tag) -> Option<f64> {
        for vertex in &self.vertex {
            if let Some(value) = vertex.unit.get(parameter) {
                return Some(value);
            }
        }
        None
    }
}
