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
    pub shadow: bool,
    #[serde(rename = "topLevel")]
    pub top_level: bool,
}

#[derive(Debug)]
pub struct Target {
    pub variables: HashMap<String, (String, usize)>,
    pub scripts: Vec<Block>,
}

impl Target {
    pub fn hydrate(i: TargetInfo) -> Self {
        let mut scripts = vec![];
        for b in i.blocks.values() {
            if b.top_level {
                scripts.push(build_block(b, &i.blocks));
            }
        }
        Target {
            variables: i.variables,
            scripts,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Value {
    Number(f64),
    String(String),
    Load(String),
}

impl Value {
    pub fn hydrate(v: &serde_json::Value) -> Value {
        let handle = |_is_shadow, v: &serde_json::Value| {
            let kind = v[0].as_u64().unwrap();
            match kind {
                4 | 5 | 6 | 7 => Value::Number(v[1].as_str().unwrap().parse().unwrap()),
                10 => Value::String(v[1].as_str().unwrap().to_owned()),
                12 => Value::Load(v[2].as_str().unwrap().to_owned()),
                _ => panic!("{:?}", v),
            }
        };
        let kind = v[0].as_u64().unwrap();
        let (block, _shadow) = match kind {
            1 => (Some(handle(true, &v[1])), None),
            2 => (Some(handle(false, &v[1])), None),
            _ => (Some(handle(false, &v[1])), Some(handle(true, &v[2]))),
        };

        block.unwrap()
    }

    pub fn as_number(&self) -> f64 {
        match self {
            Value::Number(n) => *n,
            _ => unreachable!(),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Value::String(s) => s,
            _ => unreachable!(),
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
        condition: BlockExpression,
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
}

#[derive(Debug, Clone)]
pub struct Block {
    pub op: BlockOp,
    pub next: Option<Box<Block>>,
}

fn build_block(b: &BlockInfo, blocks: &HashMap<String, BlockInfo>) -> Block {
    let op = match b.opcode.as_str() {
        "control_repeat" => BlockOp::ControlRepeat {
            times: Value::hydrate(&b.inputs["TIMES"]),
            body: Box::new(build_block(
                &blocks[b.inputs["SUBSTACK"][1].as_str().unwrap()],
                blocks,
            )),
        },
        "control_forever" => BlockOp::ControlForever(Box::new(build_block(
            &blocks[b.inputs["SUBSTACK"][1].as_str().unwrap()],
            blocks,
        ))),
        "control_wait" => BlockOp::ControlWait(Value::hydrate(&b.inputs["DURATION"])),
        "control_if_else" => BlockOp::ControlIfElse {
            condition: build_block_expr(
                &blocks[b.inputs["CONDITION"][1].as_str().unwrap()],
                blocks,
            ),
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
        "looks_say" => BlockOp::LooksSay(Value::hydrate(&b.inputs["MESSAGE"])),
        "looks_sayforsecs" => {
            return Block {
                op: BlockOp::LooksSay(Value::hydrate(&b.inputs["MESSAGE"])),
                next: Some(Box::new(Block {
                    op: BlockOp::ControlWait(Value::hydrate(&b.inputs["SECS"])),
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
            value: Value::hydrate(&b.inputs["VALUE"]),
        },
        "data_changevariableby" => BlockOp::DataChangeVariableBy {
            id: b.fields["VARIABLE"][1].as_str().unwrap().to_owned(),
            value: Value::hydrate(&b.inputs["VALUE"]),
        },
        _ => panic!("{:?}", b),
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
}

fn build_block_expr(b: &BlockInfo, _blocks: &HashMap<String, BlockInfo>) -> BlockExpression {
    match b.opcode.as_str() {
        "operator_equals" => BlockExpression::OperatorEquals {
            left: Value::hydrate(&b.inputs["OPERAND1"]),
            right: Value::hydrate(&b.inputs["OPERAND2"]),
        },
        _ => panic!("{:?}", b),
    }
}
