//! Never crash (AGENTS.md §3): bad parameters, cameras, meshes and devices
//! are errors with a reason, and the same errors as the CPU renderer's where
//! it has one. Also the embeddable way in: a renderer on someone else's
//! device, drawing into someone else's texture.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{offscreens, params};
use craft3d_gpu::{Camera, GpuError, Mesh, Projection, RenderError, RenderParams, Renderer};
use craft3d_math::Vec3;

fn unit_square(z: f64, face: u32, mesh: &mut Mesh) {
    let base = mesh.positions.len() as u32;
    mesh.positions.extend([
        Vec3::new(0.0, 0.0, z),
        Vec3::new(1.0, 0.0, z),
        Vec3::new(1.0, 1.0, z),
        Vec3::new(0.0, 1.0, z),
    ]);
    mesh.normals.extend([Vec3::ZERO; 4]);
    mesh.triangles.push([base, base + 1, base + 2]);
    mesh.triangles.push([base, base + 2, base + 3]);
    mesh.face_ids.extend([face, face]);
}

fn top_down() -> Camera {
    Camera {
        eye: Vec3::new(0.5, 0.5, 5.0),
        target: Vec3::new(0.5, 0.5, 0.0),
        up: Vec3::Y,
        projection: Projection::Orthographic { half_height: 1.0 },
    }
}

/// Every input the CPU renderer's own "bad input" test uses: the GPU
/// renderer answers with the very same error.
#[test]
fn bad_input_is_the_cpu_renderers_error_not_a_panic() {
    let mut mesh = Mesh::default();
    unit_square(0.0, 0, &mut mesh);
    let cam = top_down();
    let max = craft3d_render::MAX_SIDE;
    let mut cases: Vec<(Mesh, Camera, RenderParams)> = vec![
        (mesh.clone(), cam, params(0, 10, 1)),
        (mesh.clone(), cam, params(10, 0, 1)),
        (mesh.clone(), cam, params(max + 1, 10, 1)),
        (mesh.clone(), cam, params(10, 10, 0)),
        (mesh.clone(), cam, params(10, 10, 9)),
        (mesh.clone(), cam, params(max, max, 4)),
        (mesh.clone(), cam, params(u32::MAX, u32::MAX, u32::MAX)),
    ];
    for bad in [
        Camera {
            target: cam.eye,
            ..cam
        },
        Camera { up: Vec3::Z, ..cam },
        Camera {
            projection: Projection::Orthographic { half_height: 0.0 },
            ..cam
        },
        Camera {
            projection: Projection::Orthographic {
                half_height: f64::INFINITY,
            },
            ..cam
        },
        Camera {
            projection: Projection::Perspective { fov_y_deg: 180.0 },
            ..cam
        },
        Camera {
            projection: Projection::Perspective {
                fov_y_deg: f64::NAN,
            },
            ..cam
        },
        Camera {
            eye: Vec3::new(f64::NAN, 0.0, 0.0),
            ..cam
        },
        Camera {
            up: Vec3::new(f64::NAN, f64::NAN, f64::NAN),
            ..cam
        },
    ] {
        cases.push((mesh.clone(), bad, params(10, 10, 1)));
    }
    let mut broken = mesh.clone();
    broken.triangles[1] = [0, 1, 99];
    cases.push((broken, cam, params(10, 10, 1)));
    let mut mismatched = mesh.clone();
    mismatched.normals.pop();
    cases.push((mismatched, cam, params(10, 10, 1)));
    let mut unlabelled = mesh.clone();
    unlabelled.face_ids.pop();
    cases.push((unlabelled, cam, params(10, 10, 1)));

    for mut offscreen in offscreens() {
        for (mesh, camera, params) in &cases {
            let expected = craft3d_render::render(mesh, camera, params)
                .expect_err("the CPU renderer rejects this input");
            assert_eq!(
                offscreen.render(mesh, camera, params),
                Err(GpuError::Render(expected.clone())),
                "{expected} on {}",
                offscreen.adapter()
            );
        }
        // The renderer still works afterwards.
        let frame = offscreen.render(&mesh, &cam, &params(20, 20, 1)).unwrap();
        assert_eq!(frame.coverage(), 0.25);
    }
}

#[test]
fn an_empty_or_hostile_mesh_draws_nothing_or_something_but_never_panics() {
    let cam = top_down();
    let mut wild = Mesh::default();
    let base = wild.positions.len() as u32;
    wild.positions.extend([
        Vec3::new(-1e300, -1e300, 0.0),
        Vec3::new(1e300, -1e300, 0.0),
        Vec3::new(f64::NAN, 1e300, 0.0),
        Vec3::new(0.0, 0.0, 10.0),
        Vec3::new(f64::INFINITY, 0.0, 0.0),
        Vec3::new(1e30, -1e30, 1e30),
    ]);
    wild.normals.extend([
        Vec3::new(f64::NAN, 0.0, 0.0),
        Vec3::new(1e300, 1e300, 1e300),
        Vec3::ZERO,
        Vec3::Z,
        Vec3::new(f64::INFINITY, 0.0, 0.0),
        Vec3::X,
    ]);
    wild.triangles.extend([
        [base, base + 1, base + 2],
        [base, base + 2, base + 3],
        [base + 3, base + 4, base + 5],
        [base + 3, base + 3, base + 3],
        [base + 5, base + 3, base + 5],
    ]);
    wild.face_ids.extend([0, 1, 2, 3, u32::MAX]);

    for mut offscreen in offscreens() {
        // Nothing to draw: exactly the CPU renderer's frame, with the
        // background everywhere, no face and an infinite distance.
        for size in [(4, 4, 1), (37, 5, 3), (1, 1, 4), (300, 2, 2)] {
            let params = params(size.0, size.1, size.2);
            let cpu = craft3d_render::render(&Mesh::default(), &cam, &params).unwrap();
            let gpu = offscreen.render(&Mesh::default(), &cam, &params).unwrap();
            assert_eq!(gpu, cpu, "{size:?} on {}", offscreen.adapter());
            assert!(gpu.depth.iter().all(|d| d.is_infinite()));
        }
        for projection in [
            Projection::Orthographic { half_height: 1.0 },
            Projection::Perspective { fov_y_deg: 50.0 },
        ] {
            let camera = Camera { projection, ..cam };
            let frame = offscreen
                .render(&wild, &camera, &params(16, 16, 2))
                .unwrap();
            assert_eq!(frame.ids.len(), 256);
            // A camera inside, behind and far away from a model.
            let mut square = Mesh::default();
            unit_square(0.0, 0, &mut square);
            for eye in [
                Vec3::new(0.5, 0.5, 0.0),
                Vec3::new(0.5, 0.5, -3.0),
                Vec3::new(0.5, 0.5, 1e12),
                Vec3::new(0.5, 0.5, 1e-12),
            ] {
                let camera = Camera {
                    eye,
                    target: eye - Vec3::Z,
                    ..camera
                };
                assert!(
                    offscreen
                        .render(&square, &camera, &params(16, 16, 1))
                        .is_ok()
                );
            }
        }
    }
}

#[test]
fn numbers_and_sizes_the_gpu_cannot_take_are_errors() {
    let mut mesh = Mesh::default();
    unit_square(0.0, 0, &mut mesh);
    for mut offscreen in offscreens() {
        // Fine for the CPU renderer (2^20 samples), too wide for a texture.
        let limit = offscreen
            .renderer()
            .device()
            .limits()
            .max_texture_dimension_2d;
        let wide = params(craft3d_render::MAX_SIDE, 8, 4);
        if u64::from(limit) < 4 * u64::from(craft3d_render::MAX_SIDE) {
            assert_eq!(
                offscreen.render(&mesh, &top_down(), &wide),
                Err(GpuError::TargetTooLarge {
                    side: 4 * u64::from(craft3d_render::MAX_SIDE),
                    limit
                })
            );
        }
        // A camera that f64 can hold and f32 cannot.
        let far = Camera {
            eye: Vec3::new(0.5, 0.5, 1e100),
            ..top_down()
        };
        assert_eq!(
            offscreen.render(&mesh, &far, &params(8, 8, 1)),
            Err(GpuError::OutOfRange("the camera"))
        );
        let tiny = Camera {
            projection: Projection::Orthographic {
                half_height: 1e-300,
            },
            ..top_down()
        };
        assert_eq!(
            offscreen.render(&mesh, &tiny, &params(8, 8, 1)),
            Err(GpuError::OutOfRange("the camera"))
        );
        // Every error has a message that says what to do.
        let message = GpuError::NoAdapter("none".to_string()).to_string();
        assert!(message.contains("lavapipe") && message.contains("mesa-vulkan-drivers"));
        // A large image goes through the padded readback rows intact.
        let frame = offscreen
            .render(&mesh, &top_down(), &params(2000, 1500, 1))
            .unwrap();
        let cpu = craft3d_render::render(&mesh, &top_down(), &params(2000, 1500, 1)).unwrap();
        assert_eq!(frame.ids, cpu.ids);
        assert_eq!(frame.rgb, cpu.rgb);
        // Every distance is 5, to within the f32 arithmetic on the way
        // (sixteen roundings of 2^-24: see common/mod.rs).
        for (gpu, cpu) in frame.depth.iter().zip(&cpu.depth) {
            assert!(
                gpu == cpu || (gpu - cpu).abs() <= 5.0 * 16.0 / 16_777_216.0,
                "{gpu} is not {cpu}"
            );
        }
    }
}

/// A device for the embedding tests, made the way a shell would make one.
fn devices() -> Vec<(wgpu::Device, wgpu::Queue, String)> {
    let adapters = craft3d_gpu::adapters();
    assert!(
        !adapters.is_empty(),
        "no GPU adapter: {}",
        craft3d_gpu::ADAPTER_HINT
    );
    adapters
        .iter()
        .map(|adapter| {
            let (device, queue) =
                pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                    .unwrap();
            (device, queue, adapter.get_info().name)
        })
        .collect()
}

/// The bytes of a four-bytes-per-pixel texture.
fn read_texture(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let (width, height) = (texture.width(), texture.height());
    let stride = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(stride) * u64::from(height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    buffer.map_async(wgpu::MapMode::Read, .., |result| result.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let view = buffer.get_mapped_range(..).unwrap();
    view.chunks(stride as usize)
        .flat_map(|row| row[..width as usize * 4].to_vec())
        .collect()
}

/// The embeddable way in: someone else's device, a mesh uploaded once and
/// drawn from several cameras, and the picture copied into a texture the
/// caller owns, whatever its format.
#[test]
fn a_renderer_on_a_borrowed_device_draws_into_a_borrowed_texture() {
    let mut mesh = Mesh::default();
    unit_square(0.0, 3, &mut mesh);
    unit_square(1.0, 4, &mut mesh);
    for (device, queue, name) in devices() {
        let mut renderer = Renderer::new(&device, &queue).unwrap();
        assert_eq!(renderer.read_frame(), Err(GpuError::NothingDrawn), "{name}");
        assert!(renderer.color_view().is_none() && renderer.size().is_none());
        let uploaded = renderer.upload(&mesh).unwrap();
        assert_eq!(uploaded.triangle_count(), 4);

        let params = params(96, 64, 2);
        let cameras = [
            top_down(),
            Camera {
                eye: Vec3::new(3.0, -2.0, 4.0),
                projection: Projection::Perspective { fov_y_deg: 40.0 },
                ..top_down()
            },
        ];
        for camera in &cameras {
            renderer.draw(&uploaded, camera, &params).unwrap();
            assert!(renderer.color_view().is_some());
            assert_eq!(renderer.size(), Some((96, 64)));
            let frame = renderer.read_frame().unwrap();
            let cpu = craft3d_render::render(&mesh, camera, &params).unwrap();
            let different = frame
                .ids
                .iter()
                .zip(&cpu.ids)
                .filter(|(a, b)| a != b)
                .count();
            assert!(
                different <= 96 * 64 / 100,
                "{different} ids differ on {name}"
            );
            assert!(frame.visible_faces().contains(&4));

            for format in [
                wgpu::TextureFormat::Rgba8Unorm,
                wgpu::TextureFormat::Bgra8Unorm,
                wgpu::TextureFormat::Rgba8UnormSrgb,
            ] {
                let target = device.create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: 96,
                        height: 64,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let view = target.create_view(&wgpu::TextureViewDescriptor::default());
                renderer
                    .draw_into(&uploaded, camera, &params, &view, format)
                    .unwrap();
                let shown = read_texture(&device, &queue, &target);
                let expected = renderer.read_frame().unwrap();
                assert_eq!(expected, frame, "drawing again changes nothing");
                // The bytes in the caller's texture are the picture's: exact
                // for a plain format, within the sRGB encoder's rounding
                // (one level) for an sRGB one.
                let slack = u8::from(format.is_srgb());
                for (pixel, rgb) in shown.chunks(4).zip(expected.rgb.chunks(3)) {
                    let pixel = match format {
                        wgpu::TextureFormat::Bgra8Unorm => [pixel[2], pixel[1], pixel[0]],
                        _ => [pixel[0], pixel[1], pixel[2]],
                    };
                    for (got, want) in pixel.iter().zip(rgb) {
                        assert!(got.abs_diff(*want) <= slack, "{format:?} on {name}");
                    }
                    assert_eq!(pixel.len(), 3);
                }
            }
        }

        // A failed draw leaves no stale frame behind.
        let degenerate = Camera {
            target: cameras[0].eye,
            ..cameras[0]
        };
        assert_eq!(
            renderer.draw(&uploaded, &degenerate, &params),
            Err(GpuError::Render(RenderError::DegenerateCamera))
        );
        assert_eq!(renderer.read_frame(), Err(GpuError::NothingDrawn));
        assert!(renderer.color_view().is_none());
        renderer.draw(&uploaded, &cameras[0], &params).unwrap();
        assert!(renderer.read_frame().is_ok());
    }
}

/// What wgpu would otherwise panic about: a target format the picture
/// cannot be copied into, a mesh from another device, a destroyed device.
#[test]
fn misuse_of_the_device_is_an_error_not_a_panic() {
    let mut mesh = Mesh::default();
    unit_square(0.0, 3, &mut mesh);
    let params = params(32, 32, 1);
    for ((device, queue, name), (other_device, other_queue, _)) in
        devices().into_iter().zip(devices())
    {
        let mut renderer = Renderer::new(&device, &queue).unwrap();
        let uploaded = renderer.upload(&mesh).unwrap();

        // An integer target cannot take a colour.
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 32,
                height: 32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let refused = renderer.draw_into(
            &uploaded,
            &top_down(),
            &params,
            &view,
            wgpu::TextureFormat::R32Uint,
        );
        assert!(
            matches!(refused, Err(GpuError::Device(_))),
            "{refused:?} on {name}"
        );
        // The format says one thing and the view another.
        let refused = renderer.draw_into(
            &uploaded,
            &top_down(),
            &params,
            &view,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        assert!(
            matches!(refused, Err(GpuError::Device(_))),
            "{refused:?} on {name}"
        );
        renderer.draw(&uploaded, &top_down(), &params).unwrap();

        // A mesh that lives on another device.
        let other = Renderer::new(&other_device, &other_queue).unwrap();
        let foreign = other.upload(&mesh).unwrap();
        let refused = renderer.draw(&foreign, &top_down(), &params);
        assert_eq!(refused, Err(GpuError::ForeignMesh), "{name}");
        assert_eq!(renderer.read_frame(), Err(GpuError::NothingDrawn));
        renderer.draw(&uploaded, &top_down(), &params).unwrap();
        assert_eq!(renderer.read_frame().unwrap().coverage(), 0.25);

        // The device goes away under the renderer.
        device.destroy();
        let after = renderer
            .draw(&uploaded, &top_down(), &params)
            .and_then(|()| renderer.read_frame());
        assert!(after.is_err(), "a destroyed device still renders on {name}");
        assert!(renderer.upload(&mesh).is_err() || renderer.read_frame().is_err());
    }
}

/// Marks the child process of the test below.
const CHILD: &str = "CRAFT3D_GPU_TEST_WITHOUT_ADAPTER";

/// A machine without an adapter: the renderer says so with the way out, and
/// the tests' own way in fails rather than skips. The environment decides
/// which adapters wgpu sees, so this runs in a child process: this same
/// test, with an adapter name nothing has, then with a backend this
/// platform lacks.
#[test]
fn without_an_adapter_the_renderer_explains_and_the_tests_fail() {
    if std::env::var(CHILD).is_err() {
        let absent_backend = if cfg!(windows) { "metal" } else { "dx12" };
        for (variable, value) in [
            ("WGPU_ADAPTER_NAME", "no adapter has this name"),
            ("WGPU_BACKEND", absent_backend),
        ] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "without_an_adapter_the_renderer_explains_and_the_tests_fail",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env_remove("WGPU_ADAPTER_NAME")
                .env_remove("WGPU_BACKEND")
                .env(variable, value)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            // The child ran this very test, and it passed.
            assert!(
                output.status.success() && stdout.contains("1 passed"),
                "{variable}={value}:\n{stdout}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return;
    }
    assert!(craft3d_gpu::adapters().is_empty());
    let error = match craft3d_gpu::Offscreen::new() {
        Err(error) => error,
        Ok(offscreen) => panic!("found {} after all", offscreen.adapter()),
    };
    assert!(matches!(error, GpuError::NoAdapter(_)), "{error:?}");
    let message = error.to_string();
    assert!(
        message.contains("lavapipe") && message.contains("mesa-vulkan-drivers"),
        "{message}"
    );
    let mut mesh = Mesh::default();
    unit_square(0.0, 0, &mut mesh);
    assert!(matches!(
        craft3d_gpu::render(&mesh, &top_down(), &params(8, 8, 1)),
        Err(GpuError::NoAdapter(_))
    ));
    // The parity tests' way in panics, with the same advice.
    let panic = std::panic::catch_unwind(|| offscreens().len()).expect_err("must not skip");
    let text = panic.downcast_ref::<String>().cloned().unwrap_or_default();
    assert!(
        text.contains("no GPU adapter") && text.contains("lavapipe"),
        "{text}"
    );
}
