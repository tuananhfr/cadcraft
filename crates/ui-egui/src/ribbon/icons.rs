//! Icons for ribbon commands that no tool bar or Tool Sets entry draws, so large ribbon buttons
//! never fall back to bare text.

use crate::icons::Icon;

pub(crate) fn fallback(id: &str) -> Option<Icon> {
    use Icon::*;
    Some(match id {
        "array" => ArrayRect,
        "properties" => Properties,
        "xattach" | "xref" | "attach" => Attach,
        "count" => Count,
        "countlist" => List,
        "audit" => Audit,
        "qselect" => QuickSelect,
        "point" => Point,
        "align" => Move,
        "pedit" => Polyline,
        "dimstyle" | "dimedit" | "dimtedit" | "dimspace" | "dimbreak" | "dimoverride" | "dimreassociate" | "dimdisassociate" => DimLinear,
        "measuregeom" | "units" => Measure,
        "id" | "list" => List,
        "dist" => Distance,
        "area" | "massprop" => Area,
        "layer" | "layer.new" => LayerProps,
        "layer.current" => MakeCurrent,
        "layuniso" | "layiso" => LayerIso,
        "layoff" => LayerOff,
        "layon" => Bulb,
        "layfrz" => LayerFreeze,
        "laythw" => Sun,
        "laylck" => LayerLock,
        "layulk" => Unlock,
        "laymch" | "laymrg" => LayerMatch,
        "laydel" | "purge" | "overkill" => Erase,
        "layerstate.save" | "layerstate.restore" => LayerPrev,
        "wblock" | "block" => BlockCreate,
        "insert" | "blockreplace" => Insert,
        "dataextraction" | "datalink" | "eattext" => Table,
        "color" => Color,
        "linetype" => Linetype,
        "lweight" => Lineweight,
        "selectsimilar" => PickAdd,
        "selectall" => SelectObjects,
        "zoom.previous" | "zoom.window" => ZoomWindow,
        "zoom.all" | "zoom.extents" | "zoom.in" | "zoom.out" | "zoom.realtime" => ZoomExtents,
        "vports" | "mview" | "viewport.set" => Viewports,
        "view" => Orbit,
        "regen" | "redraw" => Regen,
        "layout.new" | "layout.copy" | "layout.rename" | "layout.delete" | "layout.set" | "pspace" | "mspace" => Layout,
        "vpclip" => Trim,
        "vplayer" => Layers,
        "ubplot" | "plot" => Plot,
        "pdfbook" | "publish" => Publish,
        "preview" => Preview,
        "pagesetup" => PageSetup,
        "compare" => Compare,
        "options" | "stylesmanager" => Gear,
        "dsettings" => Snap,
        "fonts" | "aliasedit" => Text,
        "hatchedit" | "hatchpick" | "hatchtoback" => Hatch,
        "refedit" | "bedit" => BlockEdit,
        "attsync" | "battman" => AttDef,
        "ubfill" => UbotFill,
        "ubzone" => UbotZone,
        "ubreplace" => UbotReplace,
        "tubab" => UbotSwap,
        "ubcount" => UbotCount,
        "ublegend" => UbotLegend,
        "ubcheck" => UbotCheck,
        "ublib" => UbotCatalog,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::fallback;

    #[test]
    fn every_ubot_command_has_its_own_icon() {
        let ids = ["ubfill", "ubzone", "ubreplace", "tubab", "ubcount", "ublegend", "ubcheck", "ublib"];
        let icons: Vec<_> = ids.iter().map(|id| fallback(id)).collect();
        assert!(icons.iter().all(Option::is_some), "{icons:?}");
        let distinct: std::collections::BTreeSet<_> = icons.iter().map(|icon| format!("{icon:?}")).collect();
        assert_eq!(distinct.len(), ids.len());
        let shared = ["count", "countlist", "audit", "list", "dataextraction", "array", "hatchedit", "blockreplace", "insert"];
        assert!(shared.iter().filter_map(|id| fallback(id)).all(|icon| !icons.contains(&Some(icon))));
    }
}
