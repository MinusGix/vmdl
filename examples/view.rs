use std::env::args_os;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use three_d::*;
use vmdl::mdl::Mdl;
use vmdl::vtx::Vtx;
use vmdl::vvd::Vvd;
use vmdl::{Model, Vector};

#[derive(Debug, Error)]
enum Error {
    #[error(transparent)]
    Three(#[from] Box<dyn std::error::Error>),
    #[error(transparent)]
    Mdl(#[from] vmdl::ModelError),
    #[error(transparent)]
    IO(#[from] std::io::Error),
    #[error(transparent)]
    Render(#[from] RendererError),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum DebugType {
    POSITION,
    NORMAL,
    COLOR,
    DEPTH,
    ORM,
    UV,
    NONE,
}

fn main() -> Result<(), Error> {
    miette::set_panic_hook();

    let mut args = args_os();
    let _ = args.next();
    let path = PathBuf::from(args.next().expect("No demo file provided"));
    let model = load(&path).unwrap();

    let window = Window::new(WindowSettings {
        title: path.display().to_string(),
        min_size: (512, 512),
        max_size: Some((1920, 1080)),
        ..Default::default()
    })
    .unwrap();
    let context = window.gl();

    let mut camera = Camera::new_perspective(
        window.viewport(),
        vec3(2.0, 2.0, 5.0),
        vec3(0.0, 0.0, 0.0),
        vec3(0.0, 1.0, 0.0),
        degrees(90.0),
        0.01,
        300.0,
    );

    let mut control = OrbitControl::new(*camera.target(), 1.0, 100.0);
    let mut gui = three_d::GUI::new(&context);

    let cpu_mesh = model_to_mesh(&model);
    let ph_material = PhysicalMaterial {
        albedo: Color {
            r: 128,
            g: 128,
            b: 128,
            a: 255,
        },
        ..Default::default()
    };
    let material = CpuMaterial {
        albedo: Color {
            r: 128,
            g: 128,
            b: 128,
            a: 255,
        },
        ..Default::default()
    };

    let model: three_d::Model<PhysicalMaterial> = three_d::Model::new(
        &context,
        &CpuModel {
            materials: vec![material],
            geometries: vec![cpu_mesh],
        },
    )?;

    let mut directional = [
        DirectionalLight::new(&context, 1.0, Color::WHITE, &vec3(1.0, -1.0, 0.0)),
        DirectionalLight::new(&context, 1.0, Color::WHITE, &vec3(1.0, 1.0, 0.0)),
    ];
    let mut ambient = AmbientLight {
        color: Color::WHITE,
        intensity: 0.2,
        ..Default::default()
    };

    // main loop
    let mut shadows_enabled = true;
    let mut directional_intensity = directional[0].intensity;
    let mut depth_max = 30.0;
    let mut fov = 60.0;
    let mut debug_type = DebugType::NONE;

    window.render_loop(move |mut frame_input| {
        let mut change = frame_input.first_frame;
        let mut panel_width = frame_input.viewport.width;
        change |= gui.update(
            &mut frame_input.events,
            frame_input.accumulated_time,
            frame_input.viewport,
            frame_input.device_pixel_ratio,
            |gui_context| {
                use three_d::egui::*;
                SidePanel::left("side_panel").show(gui_context, |ui| {
                    ui.heading("Debug Panel");

                    ui.label("Light options");
                    ui.add(
                        Slider::new(&mut ambient.intensity, 0.0..=1.0).text("Ambient intensity"),
                    );
                    ui.add(
                        Slider::new(&mut directional_intensity, 0.0..=1.0)
                            .text("Directional intensity"),
                    );
                    directional[0].intensity = directional_intensity;
                    directional[1].intensity = directional_intensity;
                    if ui.checkbox(&mut shadows_enabled, "Shadows").clicked() {
                        if !shadows_enabled {
                            directional[0].clear_shadow_map();
                            directional[1].clear_shadow_map();
                        }
                    }

                    ui.label("Debug options");
                    ui.radio_value(&mut debug_type, DebugType::NONE, "None");
                    ui.radio_value(&mut debug_type, DebugType::POSITION, "Position");
                    ui.radio_value(&mut debug_type, DebugType::NORMAL, "Normal");
                    ui.radio_value(&mut debug_type, DebugType::COLOR, "Color");
                    ui.radio_value(&mut debug_type, DebugType::DEPTH, "Depth");
                    ui.radio_value(&mut debug_type, DebugType::ORM, "ORM");

                    ui.label("View options");
                    ui.add(Slider::new(&mut depth_max, 1.0..=30.0).text("Depth max"));
                    ui.add(Slider::new(&mut fov, 45.0..=90.0).text("FOV"));

                    ui.label("Position");
                    ui.add(Label::new(format!("\tx: {}", camera.position().x)));
                    ui.add(Label::new(format!("\ty: {}", camera.position().y)));
                    ui.add(Label::new(format!("\tz: {}", camera.position().z)));
                });
                panel_width = gui_context.used_size().x as u32;
            },
        );

        let viewport = Viewport {
            x: panel_width as i32,
            y: 0,
            width: frame_input.viewport.width - panel_width,
            height: frame_input.viewport.height,
        };
        change |= camera.set_viewport(viewport);
        change |= control.handle_events(&mut camera, &mut frame_input.events);

        // Draw
        {
            camera.set_perspective_projection(degrees(fov), camera.z_near(), camera.z_far());
            if shadows_enabled {
                directional[0].generate_shadow_map(1024, model.iter().map(|gm| &gm.geometry));
                directional[1].generate_shadow_map(1024, model.iter().map(|gm| &gm.geometry));
            }

            let lights = &[&ambient as &dyn Light, &directional[0], &directional[1]];

            // Light pass
            let screen = frame_input.screen();
            let target = screen.clear(ClearState::default());
            match debug_type {
                DebugType::NORMAL => target.render_with_material(
                    &NormalMaterial::from_physical_material(&ph_material),
                    &camera,
                    model.iter().map(|gm| &gm.geometry),
                    lights,
                ),
                DebugType::DEPTH => {
                    let mut depth_material = DepthMaterial::default();
                    depth_material.max_distance = Some(depth_max);
                    target.render_with_material(
                        &depth_material,
                        &camera,
                        model.iter().map(|gm| &gm.geometry),
                        lights,
                    )
                }
                DebugType::ORM => target.render_with_material(
                    &ORMMaterial::from_physical_material(&ph_material),
                    &camera,
                    model.iter().map(|gm| &gm.geometry),
                    lights,
                ),
                DebugType::POSITION => {
                    let position_material = PositionMaterial::default();
                    target.render_with_material(
                        &position_material,
                        &camera,
                        model.iter().map(|gm| &gm.geometry),
                        lights,
                    )
                }
                DebugType::UV => {
                    let uv_material = UVMaterial::default();
                    target.render_with_material(
                        &uv_material,
                        &camera,
                        model.iter().map(|gm| &gm.geometry),
                        lights,
                    )
                }
                DebugType::COLOR => target.render_with_material(
                    &ColorMaterial::from_physical_material(&ph_material),
                    &camera,
                    model.iter().map(|gm| &gm.geometry),
                    lights,
                ),
                DebugType::NONE => target.render(&camera, &model, lights),
            }
            .write(|| gui.render());
        }

        let _ = change;

        FrameOutput::default()
    });
    Ok(())
}

fn load(path: &Path) -> Result<Model, vmdl::ModelError> {
    let data = fs::read(path)?;
    let mdl = Mdl::read(&data)?;
    let data = fs::read(path.with_extension("dx90.vtx"))?;
    let vtx = Vtx::read(&data)?;
    let data = fs::read(path.with_extension("vvd"))?;
    let vvd = Vvd::read(&data)?;

    Ok(Model::from_parts(mdl, vtx, vvd))
}

// 1 hammer unit is ~1.905cm
const UNIT_SCALE: f32 = 1.0 / (1.905 * 100.0);

fn model_to_mesh(model: &Model) -> CpuMesh {
    let offset = model
        .vertices()
        .iter()
        .map(|vert| vert.position.y)
        .max_by(|a, b| a.total_cmp(b))
        .unwrap();
    let offset = Vector {
        x: 0.0,
        y: -offset / 2.0,
        z: 0.0,
    };

    let positions: Vec<Vec3> = model
        .vertices()
        .iter()
        .map(|vertex| ((vertex.position + offset) * UNIT_SCALE * 10.0).into())
        .collect();
    let normals: Vec<Vec3> = model
        .vertices()
        .iter()
        .map(|vertex| vertex.normal.into())
        .collect();
    let indices = Indices::U32(
        model
            .vertex_strip_indices()
            .flat_map(|strip| strip.map(|index| index as u32))
            .collect(),
    );

    CpuMesh {
        positions: Positions::F32(positions),
        normals: Some(normals),
        indices,
        ..Default::default()
    }
}
