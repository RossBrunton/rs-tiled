use std::{collections::HashMap, sync::Arc};

use xml::attribute::OwnedAttribute;

use crate::{
    parse_properties,
    util::{get_attrs, map_wrapper, parse_tag, XmlEventResult},
    Gid, Map, MapTilesetGid, Properties, Tile, TileId, TiledError, Tileset,
};

mod finite;
mod infinite;
mod util;

pub use finite::*;
pub use infinite::*;

/// The location of the tileset this tile is in
///
/// Tilesets can be located in either one of the map's tilesets, or a tileset specified by a template.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TilesetLocation {
    /// Index into the Map's tileset list, guaranteed to be a valid index of the map tileset container
    Map(usize),
    /// Arc of the tileset itself if and only if this is location is from a template
    Template(Arc<Tileset>),
}

/// Stores the internal tile gid about a layer tile, along with how it is flipped.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayerTileData {
    /// A valid TilesetLocation that points to a tileset that **may or may not contain** this tile.
    tileset_location: TilesetLocation,
    /// The local ID of the tile in the tileset it's in.
    id: TileId,
    flip_h: bool,
    flip_v: bool,
    flip_d: bool,
}

impl LayerTileData {
    const FLIPPED_HORIZONTALLY_FLAG: u32 = 0x80000000;
    const FLIPPED_VERTICALLY_FLAG: u32 = 0x40000000;
    const FLIPPED_DIAGONALLY_FLAG: u32 = 0x20000000;
    const ALL_FLIP_FLAGS: u32 = Self::FLIPPED_HORIZONTALLY_FLAG
        | Self::FLIPPED_VERTICALLY_FLAG
        | Self::FLIPPED_DIAGONALLY_FLAG;

    /// Creates a new [`LayerTileData`] from a [`GID`] plus its flipping bits.
    pub(crate) fn from_bits(
        bits: u32,
        tilesets: &[MapTilesetGid],
        for_tileset: Option<Arc<Tileset>>,
    ) -> Option<Self> {
        let flags = bits & Self::ALL_FLIP_FLAGS;
        let gid = Gid(bits & !Self::ALL_FLIP_FLAGS);
        let flip_d = flags & Self::FLIPPED_DIAGONALLY_FLAG == Self::FLIPPED_DIAGONALLY_FLAG; // Swap x and y axis (anti-diagonally) [flips over y = -x line]
        let flip_h = flags & Self::FLIPPED_HORIZONTALLY_FLAG == Self::FLIPPED_HORIZONTALLY_FLAG; // Flip tile over y axis
        let flip_v = flags & Self::FLIPPED_VERTICALLY_FLAG == Self::FLIPPED_VERTICALLY_FLAG; // Flip tile over x axis

        if gid == Gid::EMPTY {
            None
        } else {
            let (tileset_index, tileset) = crate::util::get_tileset_for_gid(&tilesets, gid)?;
            let id = gid.0 - tileset.first_gid.0;

            Some(Self {
                tileset_location: match for_tileset {
                    None => TilesetLocation::Map(tileset_index),
                    Some(template_tileset) => {
                        // If we have an override for the tileset, it must be the tileset we found from get_tileset_for_gid
                        assert_eq!(tileset.tileset, template_tileset);
                        TilesetLocation::Template(template_tileset)
                    }
                },
                id,
                flip_h,
                flip_v,
                flip_d,
            })
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub(crate) enum TileLayerData {
    Finite(FiniteTileLayerData),
    Infinite(InfiniteTileLayerData),
}

impl TileLayerData {
    pub(crate) fn new(
        parser: &mut impl Iterator<Item = XmlEventResult>,
        attrs: Vec<OwnedAttribute>,
        infinite: bool,
        tilesets: &[MapTilesetGid],
        for_tileset: Option<Arc<Tileset>>,
    ) -> Result<(Self, Properties), TiledError> {
        let ((), (width, height)) = get_attrs!(
            attrs,
            optionals: [
            ],
            required: [
                ("width", width, |v: String| v.parse().ok()),
                ("height", height, |v: String| v.parse().ok()),
            ],
            TiledError::MalformedAttributes("layer parsing error, width and height attributes required".to_string())
        );
        let mut result = Self::Finite(Default::default());
        let mut properties = HashMap::new();
        parse_tag!(parser, "layer", {
            "data" => |attrs| {
                if infinite {
                    result = Self::Infinite(InfiniteTileLayerData::new(parser, attrs, tilesets, for_tileset.as_ref().cloned())?);
                } else {
                    result = Self::Finite(FiniteTileLayerData::new(parser, attrs, width, height, tilesets, for_tileset.as_ref().cloned())?);
                }
                Ok(())
            },
            "properties" => |_| {
                properties = parse_properties(parser)?;
                Ok(())
            },
        });

        Ok((result, properties))
    }
}

map_wrapper!(LayerTile => LayerTileData);

impl<'map> LayerTile<'map> {
    /// Get a reference to the layer tile's referenced tile, if it exists.
    pub fn get_tile(&self) -> Option<Tile<'map>> {
        self.get_tileset().get_tile(self.data.id)
    }
    /// Get a reference to the layer tile's referenced tileset.
    pub fn get_tileset(&self) -> &'map Tileset {
        match &self.data.tileset_location {
            // SAFETY: `tileset_index` is guaranteed to be valid
            TilesetLocation::Map(n) => &self.map.tilesets()[*n],
            TilesetLocation::Template(t) => &t,
        }
    }

    /// Get the layer tile's tileset index. Guaranteed to be a
    /// valid index of the map tileset container, but **isn't guaranteed to actually contain
    /// this tile**.
    ///
    /// Will return None if the tile is not located in one of the map's tilesets (i.e. the
    /// tileset is from a template).
    ///
    /// Use [`LayerTile::get_tile`] if you want to obtain the [`Tile`] that this layer tile is
    /// referencing.
    pub fn tileset_index(&self) -> Option<usize> {
        match self.data.tileset_location {
            TilesetLocation::Map(n) => Some(n),
            TilesetLocation::Template(_) => None,
        }
    }

    /// Get the layer tile's local id within its parent tileset.
    pub fn id(&self) -> u32 {
        self.data.id
    }

    /// Whether this tile is flipped on its Y axis (horizontally).
    pub fn flip_h(&self) -> bool {
        self.data.flip_h
    }

    /// Whether this tile is flipped on its X axis (vertically).
    pub fn flip_v(&self) -> bool {
        self.data.flip_v
    }

    /// Whether this tile is flipped diagonally.
    pub fn flip_d(&self) -> bool {
        self.data.flip_d
    }
}

pub enum TileLayer<'map> {
    Finite(FiniteTileLayer<'map>),
    Infinite(InfiniteTileLayer<'map>),
}

impl<'map> TileLayer<'map> {
    pub(crate) fn new(map: &'map Map, data: &'map TileLayerData) -> Self {
        match data {
            TileLayerData::Finite(data) => Self::Finite(FiniteTileLayer::new(map, data)),
            TileLayerData::Infinite(data) => Self::Infinite(InfiniteTileLayer::new(map, data)),
        }
    }

    pub fn get_tile(&self, x: i32, y: i32) -> Option<LayerTile> {
        match self {
            TileLayer::Finite(finite) => finite.get_tile(x, y),
            TileLayer::Infinite(infinite) => infinite.get_tile(x, y),
        }
    }
}
