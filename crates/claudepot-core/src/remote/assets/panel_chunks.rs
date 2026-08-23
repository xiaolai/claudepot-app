//! Lazily-loaded panel chunks — **generated**, do not edit.
//!
//! Written by `scripts/build-panel.sh` from the files Vite emitted.
//! Mermaid alone splits into dozens of diagram packs; listing them by
//! hand would be wrong within one upgrade of it.
//!
//! `remote::assets::tests::every_chunk_on_disk_is_reachable` walks the
//! directory and fails in both directions, so a rebuild that was never
//! committed and an arm whose file vanished are both caught.

/// Resolve a bare chunk filename. `None` for anything not emitted.
pub(super) fn chunk(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "abnfDiagram-VCTEODGH.js" => include_bytes!("panel/chunks/abnfDiagram-VCTEODGH.js"),
        "arc.js" => include_bytes!("panel/chunks/arc.js"),
        "architectureDiagram-5GKGNRK7.js" => {
            include_bytes!("panel/chunks/architectureDiagram-5GKGNRK7.js")
        }
        "blockDiagram-NRAW4CY4.js" => include_bytes!("panel/chunks/blockDiagram-NRAW4CY4.js"),
        "c4Diagram-UCG6FXSJ.js" => include_bytes!("panel/chunks/c4Diagram-UCG6FXSJ.js"),
        "channel.js" => include_bytes!("panel/chunks/channel.js"),
        "chunk-2Q5K7J3B.js" => include_bytes!("panel/chunks/chunk-2Q5K7J3B.js"),
        "chunk-5VM5RSS4.js" => include_bytes!("panel/chunks/chunk-5VM5RSS4.js"),
        "chunk-F27PBJKO.js" => include_bytes!("panel/chunks/chunk-F27PBJKO.js"),
        "chunk-G27WJ6UU.js" => include_bytes!("panel/chunks/chunk-G27WJ6UU.js"),
        "chunk-JWPE2WC7.js" => include_bytes!("panel/chunks/chunk-JWPE2WC7.js"),
        "chunk-LCL6LL3I.js" => include_bytes!("panel/chunks/chunk-LCL6LL3I.js"),
        "chunk-POPQ4Y6H.js" => include_bytes!("panel/chunks/chunk-POPQ4Y6H.js"),
        "chunk-SVP7TREG.js" => include_bytes!("panel/chunks/chunk-SVP7TREG.js"),
        "chunk-XXDRQBXY.js" => include_bytes!("panel/chunks/chunk-XXDRQBXY.js"),
        "classDiagram-DTDB5LWJ.js" => include_bytes!("panel/chunks/classDiagram-DTDB5LWJ.js"),
        "classDiagram-v2-JRS7N3AN.js" => include_bytes!("panel/chunks/classDiagram-v2-JRS7N3AN.js"),
        "cose-bilkent-JH36ORCC.js" => include_bytes!("panel/chunks/cose-bilkent-JH36ORCC.js"),
        "cynefin-OW5HDTMX.js" => include_bytes!("panel/chunks/cynefin-OW5HDTMX.js"),
        "cynefinDiagram-5FMLGOSQ.js" => include_bytes!("panel/chunks/cynefinDiagram-5FMLGOSQ.js"),
        "cytoscape.esm.js" => include_bytes!("panel/chunks/cytoscape.esm.js"),
        "dagre-3AP2YEHR.js" => include_bytes!("panel/chunks/dagre-3AP2YEHR.js"),
        "defaultLocale.js" => include_bytes!("panel/chunks/defaultLocale.js"),
        "diagram-S7CK7UJ4.js" => include_bytes!("panel/chunks/diagram-S7CK7UJ4.js"),
        "diagram-UQ7AKVKN.js" => include_bytes!("panel/chunks/diagram-UQ7AKVKN.js"),
        "diagram-VSXAHHWV.js" => include_bytes!("panel/chunks/diagram-VSXAHHWV.js"),
        "diagram-VX7I27RA.js" => include_bytes!("panel/chunks/diagram-VX7I27RA.js"),
        "diagram-Z3DM3KII.js" => include_bytes!("panel/chunks/diagram-Z3DM3KII.js"),
        "ebnfDiagram-PWID7BFC.js" => include_bytes!("panel/chunks/ebnfDiagram-PWID7BFC.js"),
        "erDiagram-SSCWMZ5O.js" => include_bytes!("panel/chunks/erDiagram-SSCWMZ5O.js"),
        "flowDiagram-A5DVABFB.js" => include_bytes!("panel/chunks/flowDiagram-A5DVABFB.js"),
        "ganttDiagram-EL5Y4UJY.js" => include_bytes!("panel/chunks/ganttDiagram-EL5Y4UJY.js"),
        "gitGraphDiagram-WWUBYQGX.js" => include_bytes!("panel/chunks/gitGraphDiagram-WWUBYQGX.js"),
        "infoDiagram-RXCK75RN.js" => include_bytes!("panel/chunks/infoDiagram-RXCK75RN.js"),
        "init.js" => include_bytes!("panel/chunks/init.js"),
        "ishikawaDiagram-5VMMS53U.js" => include_bytes!("panel/chunks/ishikawaDiagram-5VMMS53U.js"),
        "journeyDiagram-EYS64GPL.js" => include_bytes!("panel/chunks/journeyDiagram-EYS64GPL.js"),
        "kanban-definition-3QL26DDD.js" => {
            include_bytes!("panel/chunks/kanban-definition-3QL26DDD.js")
        }
        "katex.js" => include_bytes!("panel/chunks/katex.js"),
        "layout.js" => include_bytes!("panel/chunks/layout.js"),
        "linear.js" => include_bytes!("panel/chunks/linear.js"),
        "mermaid.core.js" => include_bytes!("panel/chunks/mermaid.core.js"),
        "Mermaid.js" => include_bytes!("panel/chunks/Mermaid.js"),
        "mindmap-definition-FBJOCRG2.js" => {
            include_bytes!("panel/chunks/mindmap-definition-FBJOCRG2.js")
        }
        "ordinal.js" => include_bytes!("panel/chunks/ordinal.js"),
        "pegDiagram-XKGWAZYB.js" => include_bytes!("panel/chunks/pegDiagram-XKGWAZYB.js"),
        "pieDiagram-E7YTZNPT.js" => include_bytes!("panel/chunks/pieDiagram-E7YTZNPT.js"),
        "quadrantDiagram-AXDQQJYC.js" => include_bytes!("panel/chunks/quadrantDiagram-AXDQQJYC.js"),
        "railroadDiagram-O6MQD6OU.js" => include_bytes!("panel/chunks/railroadDiagram-O6MQD6OU.js"),
        "requirementDiagram-EFPCY7ZU.js" => {
            include_bytes!("panel/chunks/requirementDiagram-EFPCY7ZU.js")
        }
        "sankeyDiagram-P5KCCOFB.js" => include_bytes!("panel/chunks/sankeyDiagram-P5KCCOFB.js"),
        "sequenceDiagram-WJ2MYXX4.js" => include_bytes!("panel/chunks/sequenceDiagram-WJ2MYXX4.js"),
        "sizeCapture-X5ZJPWSS.js" => include_bytes!("panel/chunks/sizeCapture-X5ZJPWSS.js"),
        "stateDiagram-HBIQ2CUA.js" => include_bytes!("panel/chunks/stateDiagram-HBIQ2CUA.js"),
        "stateDiagram-v2-4QOOHH4V.js" => include_bytes!("panel/chunks/stateDiagram-v2-4QOOHH4V.js"),
        "swimlanes-XN3QIQJK.js" => include_bytes!("panel/chunks/swimlanes-XN3QIQJK.js"),
        "swimlanesDiagram-VK2B7HYN.js" => {
            include_bytes!("panel/chunks/swimlanesDiagram-VK2B7HYN.js")
        }
        "timeline-definition-24CTP7MA.js" => {
            include_bytes!("panel/chunks/timeline-definition-24CTP7MA.js")
        }
        "vennDiagram-4TSXK5OY.js" => include_bytes!("panel/chunks/vennDiagram-4TSXK5OY.js"),
        "wardleyDiagram-VM6X3IG4.js" => include_bytes!("panel/chunks/wardleyDiagram-VM6X3IG4.js"),
        "xychartDiagram-S5SC5T6Z.js" => include_bytes!("panel/chunks/xychartDiagram-S5SC5T6Z.js"),
        _ => return None,
    })
}
