use std::collections::HashMap;

#[derive(serde::Deserialize, Debug)]
pub struct ProjectInfo {
    pub targets: Vec<TargetInfo>,
    pub extensions: Vec<String>,
    pub meta: serde_json::Value,
}

impl ProjectInfo {
    pub fn new(
        data: impl std::io::Read + std::io::Seek,
    ) -> Result<ProjectInfo, Box<dyn std::error::Error>> {
        let mut archive = zip::ZipArchive::new(data)?;
        let mut file = archive.by_name("project.json")?;
        let mut source = Vec::new();
        std::io::copy(&mut file, &mut source)?;
        Ok(serde_json::from_slice(&source)?)
    }
}

#[derive(serde::Deserialize, Debug)]
pub struct TargetInfo {
    #[serde(rename = "isStage")]
    pub is_stage: bool,
    pub name: String,
    pub variables: HashMap<String, (String, usize)>,
    pub lists: serde_json::Value,
    pub broadcasts: serde_json::Value,
    pub blocks: HashMap<String, BlockInfo>,
    pub comments: serde_json::Value,
}

#[derive(serde::Deserialize, Debug)]
pub struct BlockInfo {
    pub opcode: String,
    pub next: Option<String>,
    pub parent: Option<String>,
    pub inputs: HashMap<String, serde_json::Value>,
    pub fields: HashMap<String, serde_json::Value>,
    pub mutation: Option<MutationInfo>,
    pub shadow: bool,
    #[serde(rename = "topLevel")]
    pub top_level: bool,
}

#[derive(serde::Deserialize, Debug)]
struct ArgumentNames(#[serde(with = "serde_with::json::nested")] Vec<String>);

#[derive(serde::Deserialize, Debug)]
pub struct MutationInfo {
    argumentnames: Option<ArgumentNames>,
    #[serde(with = "serde_with::json::nested")]
    argumentids: Vec<String>,
    proccode: String,
}

#[derive(Debug)]
pub struct Target {
    pub variables: HashMap<String, (String, usize)>,
    pub scripts: Vec<Block>,
    pub procedures: Vec<Procedure>,
}

impl Target {
    pub fn hydrate(i: TargetInfo) -> Self {
        let mut scripts = vec![];
        let mut procedures = vec![];
        for b in i.blocks.values() {
            if b.opcode == "procedures_definition" {
                let body = build_block(&i.blocks[b.next.as_ref().unwrap()], &i.blocks);

                let prototype = &i.blocks[b.inputs["custom_block"][1].as_str().unwrap()];
                let MutationInfo {
                    argumentnames,
                    proccode,
                    ..
                } = prototype.mutation.as_ref().unwrap();
                procedures.push(Procedure {
                    id: proccode.to_owned(),
                    arguments: argumentnames.as_ref().unwrap().0.clone(),
                    body,
                });
            } else if b.top_level {
                scripts.push(build_block(b, &i.blocks));
            }
        }
        Target {
            variables: i.variables,
            scripts,
            procedures,
        }
    }
}

#[derive(Debug)]
pub struct Procedure {
    pub id: String,
    pub arguments: Vec<String>,
    pub body: Block,
}

#[derive(Debug, Clone)]
pub enum Value {
    Number(f64),
    String(String),
    Load(String),
    Expression(Box<BlockExpression>),
}

impl Value {
    pub fn hydrate(v: &serde_json::Value, blocks: &HashMap<String, BlockInfo>) -> Value {
        if v[1].is_array() {
            let kind = v[1][0].as_u64().unwrap();
            match kind {
                4 | 5 | 6 | 7 => Value::Number(v[1][1].as_str().unwrap().parse().unwrap()),
                10 => Value::String(v[1][1].as_str().unwrap().to_owned()),
                12 => Value::Load(v[1][2].as_str().unwrap().to_owned()),
                _ => panic!("{:#?}", v),
            }
        } else {
            Value::Expression(Box::new(build_block_expr(
                &blocks[v[1].as_str().unwrap()],
                blocks,
            )))
        }
    }
}

#[derive(Debug, Clone)]
pub enum BlockOp {
    ControlRepeat {
        times: Value,
        body: Box<Block>,
    },
    ControlForever(Box<Block>),
    ControlWait(Value),
    ControlIfElse {
        condition: Value,
        consequent: Box<Block>,
        alternative: Box<Block>,
    },
    ControlStopAll,
    ControlStopScript,
    LooksSay(Value),
    EventWhenFlagClicked,
    DataSetVariableTo {
        id: String,
        value: Value,
    },
    DataChangeVariableBy {
        id: String,
        value: Value,
    },
    ProceduresCall {
        proc: String,
        args: Vec<Value>,
    },
}

#[derive(Debug, Clone)]
pub struct Block {
    pub op: BlockOp,
    pub next: Option<Box<Block>>,
}

fn build_block(b: &BlockInfo, blocks: &HashMap<String, BlockInfo>) -> Block {
    let op = match b.opcode.as_str() {
        "control_repeat" => BlockOp::ControlRepeat {
            times: Value::hydrate(&b.inputs["TIMES"], blocks),
            body: Box::new(build_block(
                &blocks[b.inputs["SUBSTACK"][1].as_str().unwrap()],
                blocks,
            )),
        },
        "control_forever" => BlockOp::ControlForever(Box::new(build_block(
            &blocks[b.inputs["SUBSTACK"][1].as_str().unwrap()],
            blocks,
        ))),
        "control_wait" => BlockOp::ControlWait(Value::hydrate(&b.inputs["DURATION"], blocks)),
        "control_if_else" => BlockOp::ControlIfElse {
            condition: Value::hydrate(&b.inputs["CONDITION"], blocks),
            consequent: Box::new(build_block(
                &blocks[b.inputs["SUBSTACK"][1].as_str().unwrap()],
                blocks,
            )),
            alternative: Box::new(build_block(
                &blocks[b.inputs["SUBSTACK2"][1].as_str().unwrap()],
                blocks,
            )),
        },
        "control_stop" => match b.fields["STOP_OPTION"][0].as_str().unwrap() {
            "all" => BlockOp::ControlStopAll,
            "this script" => BlockOp::ControlStopScript,
            _ => unreachable!("{:?}", b),
        },
        "looks_say" => BlockOp::LooksSay(Value::hydrate(&b.inputs["MESSAGE"], blocks)),
        "looks_sayforsecs" => {
            return Block {
                op: BlockOp::LooksSay(Value::hydrate(&b.inputs["MESSAGE"], blocks)),
                next: Some(Box::new(Block {
                    op: BlockOp::ControlWait(Value::hydrate(&b.inputs["SECS"], blocks)),
                    next: if let Some(id) = &b.next {
                        Some(Box::new(build_block(&blocks[id], blocks)))
                    } else {
                        None
                    },
                })),
            };
        }
        "event_whenflagclicked" => BlockOp::EventWhenFlagClicked,
        "data_setvariableto" => BlockOp::DataSetVariableTo {
            id: b.fields["VARIABLE"][1].as_str().unwrap().to_owned(),
            value: Value::hydrate(&b.inputs["VALUE"], blocks),
        },
        "data_changevariableby" => BlockOp::DataChangeVariableBy {
            id: b.fields["VARIABLE"][1].as_str().unwrap().to_owned(),
            value: Value::hydrate(&b.inputs["VALUE"], blocks),
        },
        "procedures_call" => BlockOp::ProceduresCall {
            proc: b.mutation.as_ref().unwrap().proccode.clone(),
            args: b
                .mutation
                .as_ref()
                .unwrap()
                .argumentids
                .iter()
                .map(|id| Value::hydrate(&b.inputs[id], blocks))
                .collect(),
        },
        _ => panic!("{:#?}", b),
    };
    Block {
        op,
        next: if let Some(id) = &b.next {
            Some(Box::new(build_block(&blocks[id], blocks)))
        } else {
            None
        },
    }
}

#[derive(Debug, Clone)]
pub enum BlockExpression {
    OperatorEquals { left: Value, right: Value },
    OperatorGT { left: Value, right: Value },
    OperatorAdd { left: Value, right: Value },
    OperatorSubtract { left: Value, right: Value },
    ArgumentReporterStringNumber { name: String },
}

fn build_block_expr(b: &BlockInfo, blocks: &HashMap<String, BlockInfo>) -> BlockExpression {
    match b.opcode.as_str() {
        "operator_equals" => BlockExpression::OperatorEquals {
            left: Value::hydrate(&b.inputs["OPERAND1"], blocks),
            right: Value::hydrate(&b.inputs["OPERAND2"], blocks),
        },
        "operator_gt" => BlockExpression::OperatorGT {
            left: Value::hydrate(&b.inputs["OPERAND1"], blocks),
            right: Value::hydrate(&b.inputs["OPERAND2"], blocks),
        },
        "operator_add" => BlockExpression::OperatorAdd {
            left: Value::hydrate(&b.inputs["NUM1"], blocks),
            right: Value::hydrate(&b.inputs["NUM2"], blocks),
        },
        "operator_subtract" => BlockExpression::OperatorSubtract {
            left: Value::hydrate(&b.inputs["NUM1"], blocks),
            right: Value::hydrate(&b.inputs["NUM2"], blocks),
        },
        "argument_reporter_string_number" => BlockExpression::ArgumentReporterStringNumber {
            name: b.fields["VALUE"][0].as_str().unwrap().to_owned(),
        },
        _ => panic!("{:#?}", b),
    }
}
