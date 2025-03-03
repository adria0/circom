use super::type_definitions::*;
use circom_algebra::algebra::ArithmeticExpression;
use compiler::hir::very_concrete_program::*;
use dag::DAG;
use num_bigint::BigInt;
use program_structure::ast::{SignalType, Statement};
use std::collections::{HashMap, HashSet};

struct Connexion {
    full_name: String,
    inspect: SubComponentData,
    dag_offset: usize,
    dag_component_offset: usize,
    dag_jump: usize,
    dag_component_jump: usize,
}

pub struct ExecutedTemplate {
    pub code: Statement,
    pub template_name: String,
    pub report_name: String,
    pub inputs: SignalCollector,
    pub outputs: SignalCollector,
    pub constraints: Vec<Constraint>,
    pub intermediates: SignalCollector,
    pub components: ComponentCollector,
    pub public_inputs: HashSet<String>,
    pub parameter_instances: ParameterContext,
    pub is_parallel: bool,
    pub has_parallel_sub_cmp: bool,
    connexions: Vec<Connexion>,
}

impl ExecutedTemplate {
    pub fn new(
        public: Vec<String>,
        name: String,
        report_name: String,
        instance: ParameterContext,
        code: Statement,
        is_parallel: bool,
    ) -> ExecutedTemplate {
        let public_inputs: HashSet<_> = public.iter().cloned().collect();
        ExecutedTemplate {
            report_name,
            public_inputs,
            is_parallel,
            has_parallel_sub_cmp: false,
            code: code.clone(),
            template_name: name,
            parameter_instances: instance,
            inputs: SignalCollector::new(),
            outputs: SignalCollector::new(),
            intermediates: SignalCollector::new(),
            components: ComponentCollector::new(),
            constraints: Vec::new(),
            connexions: Vec::new(),
        }
    }

    pub fn is_equal(&self, name: &str, context: &ParameterContext) -> bool {
        self.template_name == name && self.parameter_instances == *context
    }

    pub fn add_arrow(&mut self, component_name: String, data: SubComponentData) {
        let cnn =
            Connexion { full_name: component_name, inspect: data, dag_offset: 0, dag_component_offset: 0, dag_jump: 0, dag_component_jump: 0};
        self.connexions.push(cnn);
    }

    pub fn add_input(&mut self, input_name: &str, dimensions: &[usize]) {
        self.inputs.push((input_name.to_string(), dimensions.to_vec()));
    }

    pub fn add_output(&mut self, output_name: &str, dimensions: &[usize]) {
        self.outputs.push((output_name.to_string(), dimensions.to_vec()));
    }

    pub fn add_intermediate(&mut self, intermediate_name: &str, dimensions: &[usize]) {
        self.intermediates.push((intermediate_name.to_string(), dimensions.to_vec()));
    }

    pub fn add_component(&mut self, component_name: &str, dimensions: &[usize]) {
        self.components.push((component_name.to_string(), dimensions.to_vec()));
    }

    pub fn add_constraint(&mut self, constraint: Constraint) {
        self.constraints.push(constraint);
    }

    pub fn template_name(&self) -> &String {
        &self.template_name
    }

    pub fn parameter_instances(&self) -> &ParameterContext {
        &self.parameter_instances
    }

    pub fn inputs(&self) -> &SignalCollector {
        &self.inputs
    }

    pub fn outputs(&self) -> &SignalCollector {
        &self.outputs
    }

    pub fn intermediates(&self) -> &SignalCollector {
        &self.intermediates
    }

    pub fn insert_in_dag(&mut self, dag: &mut DAG) {
        dag.add_node(self.report_name.clone(), self.is_parallel);
        self.build_signals(dag);
        self.build_connexions(dag);
        self.build_constraints(dag);
    }

    fn build_signals(&self, dag: &mut DAG) {
        for (name, dim) in self.outputs() {
            let state = State { name: name.clone(), dim: 0 };
            let config = SignalConfig { signal_type: 1, dimensions: dim, is_public: false };
            generate_symbols(dag, state, &config);
        }
        for (name, dim) in self.inputs() {
            if self.public_inputs.contains(name) {
                let state = State { name: name.clone(), dim: 0 };
                let config = SignalConfig { signal_type: 0, dimensions: dim, is_public: true };
                generate_symbols(dag, state, &config);
            }
        }
        for (name, dim) in self.inputs() {
            if !self.public_inputs.contains(name) {
                let state = State { name: name.clone(), dim: 0 };
                let config = SignalConfig { signal_type: 0, dimensions: dim, is_public: false };
                generate_symbols(dag, state, &config);
            }
        }
        for (name, dim) in self.intermediates() {
            let state = State { name: name.clone(), dim: 0 };
            let config = SignalConfig { signal_type: 2, dimensions: dim, is_public: false };
            generate_symbols(dag, state, &config);
        }
    }
    fn build_connexions(&mut self, dag: &mut DAG) {
        self.connexions.sort_by(|l, r| {
            use std::cmp::Ordering;
            let l_data = &l.inspect;
            let r_data = &r.inspect;
            let cmp_0 = l_data.name.cmp(&r_data.name);
            match cmp_0 {
                Ordering::Equal => l_data.indexed_with.cmp(&r_data.indexed_with),
                v => v,
            }
        });
        self.components = filter_used_components(self);
        for cnn in &mut self.connexions {
            cnn.dag_offset = dag.get_entry().unwrap().get_out();
            cnn.dag_component_offset = dag.get_entry().unwrap().get_out_component();
            dag.add_edge(cnn.inspect.goes_to, &cnn.full_name);
            cnn.dag_jump = dag.get_entry().unwrap().get_out() - cnn.dag_offset;
            cnn.dag_component_jump = dag.get_entry().unwrap().get_out_component() - cnn.dag_component_offset;
        }
        self.has_parallel_sub_cmp = dag.nodes[dag.main_id()].has_parallel_sub_cmp();
    }
    fn build_constraints(&self, dag: &mut DAG) {
        for c in &self.constraints {
            let correspondence = dag.get_main().unwrap().correspondence();
            let cc = Constraint::apply_correspondence(c, correspondence);
            dag.add_constraint(cc);
        }
    }
    pub fn export_to_circuit(self, instances: &[TemplateInstance]) -> TemplateInstance {
        use SignalType::*;
        fn build_triggers(
            instances: &[TemplateInstance],
            connexions: Vec<Connexion>,
        ) -> Vec<Trigger> {
            let mut triggers = vec![];
            for cnn in connexions {
                let data = cnn.inspect;
                let trigger = Trigger {
                    offset: cnn.dag_offset,
                    component_offset: cnn.dag_component_offset,
                    component_name: data.name,
                    indexed_with: data.indexed_with,
                    runs: instances[data.goes_to].template_header.clone(),
                    template_id: data.goes_to,
                    external_signals: instances[data.goes_to].signals.clone(),
                };
                triggers.push(trigger);
            }
            triggers
        }

        fn build_components(components: ComponentCollector) -> Vec<Component> {
            let mut cmp = vec![];
            for (name, lengths) in components {
                cmp.push(Component { name, lengths })
            }
            cmp
        }

        fn build_arguments(parameter_instances: ParameterContext) -> Vec<Argument> {
            let mut arguments = vec![];
            for (name, data) in parameter_instances {
                let (dim, value) = data.destruct();
                let argument = Argument { name, lengths: dim, values: as_big_int(value) };
                arguments.push(argument);
            }
            arguments
        }

        let header = format!("{}_{}", self.template_name, instances.len());
        let clusters = build_clusters(&self, instances);
        let triggers = build_triggers(instances, self.connexions);
        let components = build_components(self.components);
        let arguments = build_arguments(self.parameter_instances);
        let config = TemplateConfig {
            header,
            clusters,
            triggers,
            arguments,
            components,
            id: instances.len(),
            is_parallel: self.is_parallel,
            has_parallel_sub_cmp: self.has_parallel_sub_cmp,
            code: self.code,
            name: self.template_name,
        };

        let mut instance = TemplateInstance::new(config);

        let mut public = vec![];
        let mut not_public = vec![];
        for s in self.inputs {
            if self.public_inputs.contains(&s.0) {
                public.push(s);
            } else {
                not_public.push(s);
            }
        }
        let mut local_id = 0;
        let mut dag_local_id = 1;
        for (name, lengths) in self.outputs {
            let signal = Signal { name, lengths, local_id, dag_local_id, xtype: Output };
            local_id += signal.size();
            dag_local_id += signal.size();
            instance.add_signal(signal);
        }
        for (name, lengths) in public {
            let signal = Signal { name, lengths, local_id, dag_local_id, xtype: Input };
            local_id += signal.size();
            dag_local_id += signal.size();
            instance.add_signal(signal);
        }
        for (name, lengths) in not_public {
            let signal = Signal { name, lengths, local_id, dag_local_id, xtype: Input };
            local_id += signal.size();
            dag_local_id += signal.size();
            instance.add_signal(signal);
        }
        for (name, lengths) in self.intermediates {
            let signal = Signal { name, lengths, local_id, dag_local_id, xtype: Intermediate };
            local_id += signal.size();
            dag_local_id += signal.size();
            instance.add_signal(signal);
        }
        instance
    }
}

struct SignalConfig<'a> {
    is_public: bool,
    signal_type: usize,
    dimensions: &'a [usize],
}
struct State {
    name: String,
    dim: usize,
}
fn generate_symbols(dag: &mut DAG, state: State, config: &SignalConfig) {
    if state.dim == config.dimensions.len() {
        if config.signal_type == 0 {
            dag.add_input(state.name, config.is_public);
        } else if config.signal_type == 1 {
            dag.add_output(state.name);
        } else if config.signal_type == 2 {
            dag.add_intermediate(state.name);
        }
    } else {
        let mut index = 0;
        while index < config.dimensions[state.dim] {
            let new_state =
                State { name: format!("{}[{}]", state.name, index), dim: state.dim + 1 };
            generate_symbols(dag, new_state, config);
            index += 1;
        }
    }
}

fn as_big_int(exprs: Vec<ArithmeticExpression<String>>) -> Vec<BigInt> {
    let mut numbers = Vec::with_capacity(exprs.len());
    for e in exprs {
        if let ArithmeticExpression::Number { value } = e {
            numbers.push(value);
        }
    }
    numbers
}

fn filter_used_components(tmp: &ExecutedTemplate) -> ComponentCollector {
    let mut used = HashSet::with_capacity(tmp.components.len());
    for cnn in &tmp.connexions {
        used.insert(cnn.inspect.name.clone());
    }
    let mut filtered = Vec::with_capacity(used.len());
    for cmp in &tmp.components {
        if used.contains(&cmp.0) {
            filtered.push(cmp.clone());
        }
    }
    filtered
}

#[derive(Copy, Clone)]
enum POS {
    T,
    K(usize),
    B,
}
impl POS {
    pub fn least_upper_bound(l: POS, r: POS) -> POS {
        use POS::*;
        match (l, r) {
            (K(v0), K(v1)) if v0 == v1 => K(v0),
            (B, p) | (p, B) => p,
            _ => T,
        }
    }
}
fn apply_pos_to_connexions(connexions: &[Connexion]) -> HashMap<String, POS> {
    use POS::*;
    let mut solution = HashMap::with_capacity(connexions.len());
    for cnn in connexions {
        let name = &cnn.inspect.name;
        solution.insert(name.clone(), B);
    }
    for cnn in connexions {
        let data = &cnn.inspect;
        let prev = solution.remove(&data.name).unwrap();
        let new = K(data.goes_to);
        let val = POS::least_upper_bound(prev, new);
        solution.insert(data.name.clone(), val);
    }
    solution
}

fn mixed_components(exec_tmp: &ExecutedTemplate) -> Vec<bool> {
    use POS::*;
    let solution = apply_pos_to_connexions(&exec_tmp.connexions);
    let mut mixed = vec![false; exec_tmp.components.len()];
    for (index, value) in exec_tmp.components.iter().enumerate() {
        let pos_value = solution.get(&value.0).unwrap();
        mixed[index] = mixed[index] || matches!(pos_value, T);
    }
    mixed
}

fn build_clusters(tmp: &ExecutedTemplate, instances: &[TemplateInstance]) -> Vec<TriggerCluster> {
    let components = &tmp.components;
    let connexions = &tmp.connexions;
    let mixed = mixed_components(tmp);
    let mut result = Vec::with_capacity(components.len());

    // Cluster initialization
    let mut cmp_data = HashMap::with_capacity(components.len());
    let mut index = 0;
    while index < connexions.len() {
        let cnn_data = &connexions[index].inspect;
        let offset_jump = connexions[index].dag_jump;
        let component_offset_jump = connexions[index].dag_component_jump;
        let instance_id = connexions[index].inspect.goes_to;
        let sub_cmp_header = instances[instance_id].template_header.clone();
        let start = index;
        let mut end = index;
        loop {
            if end == connexions.len() {
                break;
            } else if connexions[end].inspect.name != cnn_data.name {
                break;
            } else {
                end += 1;
            }
        }
        let cluster = TriggerCluster {
            slice: start..end,
            length: end - start,
            cmp_name: cnn_data.name.clone(),
            xtype: ClusterType::Uniform { offset_jump, component_offset_jump, instance_id, header: sub_cmp_header },
        };
        cmp_data.insert(cnn_data.name.clone(), cluster);
        index = end;
    }

    // cmp_data and result binding
    let mut index = 0;
    while index < components.len() {
        let cmp_name = &components[index].0;
        let mut cluster = cmp_data.remove(cmp_name).unwrap();
        let start = cluster.slice.start;
        let tmp_id = connexions[start].inspect.goes_to;
        let tmp_name = instances[tmp_id].template_name.clone();
        if mixed[index] {
            cluster.xtype = ClusterType::Mixed { tmp_name };
        }
        result.push(cluster);
        index += 1;
    }
    result
}

pub fn templates_in_mixed_arrays(exec_tmp: &ExecutedTemplate, no_templates: usize) -> Vec<bool> {
    use POS::*;
    let solution = apply_pos_to_connexions(&exec_tmp.connexions);
    let mut mixed = vec![false; no_templates];
    for cnn in &exec_tmp.connexions {
        let data = &cnn.inspect;
        let pos_value = solution.get(&data.name).unwrap();
        mixed[data.goes_to] = mixed[data.goes_to] || matches!(pos_value, T);
    }
    mixed
}
