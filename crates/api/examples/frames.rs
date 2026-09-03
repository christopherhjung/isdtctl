//! Prints the on-air bytes for a few requests, for comparison against a capture.
use api::frame;
use api::types::{BatteryKind, LinkType, TaskType};
use api::Request;

fn main() {
    let cases = vec![
        ("hardware info", Request::HardwareInfo),
        ("electrical", Request::Electrical { channel: 0 }),
        (
            "charge 4S LiPo 1000mA",
            Request::SetTask {
                channel: 0,
                task: TaskType::Charge,
                battery: BatteryKind::LiPo,
                link: LinkType::SerialOnly,
                work_current_ma: 1000,
                cell_count: 4,
                full_charged_volt_mv: 4200,
            },
        ),
    ];
    for (name, request) in cases {
        let data = request.data();
        let encoded = frame::encode(&data).unwrap();
        println!("{name}");
        println!("  data  ({:2}) {}", data.len(), hex(&data));
        println!("  frame ({:2}) {}", encoded.len(), hex(&encoded));
        for (i, packet) in frame::chunk(&encoded, 20).iter().enumerate() {
            println!("  write{i} ({:2}) {}", packet.len(), hex(packet));
        }
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x} ")).collect()
}
