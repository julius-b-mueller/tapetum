//! Beams in haze as analytic cone volumes (see `beam.wesl`).
//!
//! Bevy's volumetric fog only knows point-like sources, needs a shadow map per
//! beam and cannot show gobos. A beam here starts with the diameter of the
//! LED field, costs only the pixels it covers, and samples the gobo itself.

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_asset, embedded_path};
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, CompareFunction, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError};
use bevy::render::storage::ShaderBuffer;
use bevy::shader::ShaderRef;

pub struct BeamPlugin;

impl Plugin for BeamPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "beam.wesl");
        embedded_asset!(app, "lens.wesl");
        embedded_asset!(app, "surface.wesl");
        app.add_plugins((
            MaterialPlugin::<BeamMaterial>::default(),
            MaterialPlugin::<LensMaterial>::default(),
            MaterialPlugin::<SurfaceMaterial>::default(),
        ))
        .init_resource::<SurfaceGobos>()
        .add_systems(PostUpdate, upload_surface_gobos);
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct BeamMaterial {
    #[uniform(0)]
    pub params: BeamParams,
    /// Gobo mask (R channel); a white pixel when there is none.
    #[texture(1)]
    #[sampler(2)]
    pub gobo: Handle<Image>,
}

/// Mirrors `struct Beam` in the shader.
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct BeamParams {
    pub apex: Vec4,
    pub axis: Vec4,
    pub up: Vec4,
    pub color: Vec4,
    pub haze: Vec4,
    pub tint: Vec4,
    pub lens: Vec4,
}

impl Material for BeamMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::from_path_buf(embedded_path!("beam.wesl")).with_source("embedded"))
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Back faces only: exactly one fragment per covered pixel, also when the
        // camera is inside the beam. Depth is handled in the shader, so a beam
        // whose far side is below the floor still shows in front of it.
        descriptor.primitive.cull_mode = Some(Face::Front);
        if let Some(ds) = descriptor.depth_stencil.as_mut() {
            ds.depth_compare = Some(CompareFunction::Always);
            ds.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

/// LEDs and lenses: bright towards the beam, dim from the side (see `lens.wesl`).
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct LensMaterial {
    #[uniform(0)]
    pub params: LensParams,
}

/// Mirrors `struct Lens` in the shader.
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct LensParams {
    pub color: Vec4,
    pub cone: Vec4,
    pub axis: Vec4,
}

impl Material for LensMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::from_path_buf(embedded_path!("lens.wesl")).with_source("embedded"))
    }

    fn enable_shadows() -> bool {
        false
    }
}

/// Every stage surface: standard lighting minus the light the gobos block
/// (see `surface.wesl`).
pub type SurfaceMaterial = ExtendedMaterial<StandardMaterial, SurfaceExt>;

/// Gobos the surfaces show at most.
pub const SURFACE_GOBOS: usize = 32;

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug, PartialEq)]
pub struct SurfaceExt {
    /// [`SurfaceGobos::buffer`], shared by all surfaces.
    #[storage(100, read_only)]
    pub gobos: Handle<ShaderBuffer>,
    /// All gobo masks as layers of one texture array.
    #[texture(101, dimension = "2d_array")]
    #[sampler(102)]
    pub masks: Handle<Image>,
}

impl MaterialExtension for SurfaceExt {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(AssetPath::from_path_buf(embedded_path!("surface.wesl")).with_source("embedded"))
    }
}

/// The gobos projected right now, for every surface. One buffer of fixed size,
/// so updating it rewrites it in place instead of rebinding all materials.
#[derive(Resource)]
pub struct SurfaceGobos {
    pub buffer: Handle<ShaderBuffer>,
    pub masks: Handle<Image>,
    /// Per gobo: lens (w: mask layer), axis (w: range), up (w: tan half angle),
    /// cone (x: cos half angle, y: light left in blocked parts, z: 1 = water).
    pub entries: Vec<[Vec4; 4]>,
    uploaded: Option<Vec<[Vec4; 4]>>,
}

impl SurfaceGobos {
    pub fn ext(&self) -> SurfaceExt {
        SurfaceExt { gobos: self.buffer.clone(), masks: self.masks.clone() }
    }
}

impl FromWorld for SurfaceGobos {
    fn from_world(world: &mut World) -> Self {
        let buffer = world.resource_mut::<Assets<ShaderBuffer>>().add(gobo_buffer(&[]));
        let masks = world.resource_mut::<Assets<Image>>().add(super::gobo::mask_array(&[]));
        SurfaceGobos { buffer, masks, entries: Vec::new(), uploaded: None }
    }
}

fn gobo_buffer(entries: &[[Vec4; 4]]) -> ShaderBuffer {
    let mut data = vec![[0f32; 4]; 1 + 4 * SURFACE_GOBOS];
    let n = entries.len().min(SURFACE_GOBOS);
    data[0][0] = n as f32;
    for (k, e) in entries.iter().take(n).enumerate() {
        for (j, v) in e.iter().enumerate() {
            data[1 + 4 * k + j] = v.to_array();
        }
    }
    ShaderBuffer::new(data, RenderAssetUsages::RENDER_WORLD)
}

fn upload_surface_gobos(mut gobos: ResMut<SurfaceGobos>, mut buffers: ResMut<Assets<ShaderBuffer>>) {
    if gobos.uploaded.as_ref() == Some(&gobos.entries) {
        return;
    }
    if let Some(mut b) = buffers.get_mut(&gobos.buffer) {
        *b = gobo_buffer(&gobos.entries);
    }
    let entries = gobos.entries.clone();
    gobos.uploaded = Some(entries);
}
