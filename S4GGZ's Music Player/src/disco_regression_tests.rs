mod qa_disco {
    use super::*;

    fn ray_alphas(scene: &mut DiscoScene, ctx: &egui::Context, time: f64) -> Vec<u8> {
        let mut result = vec![];
        let _ = crate::test_support::run_ui(ctx, egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 790.0))),
            time: Some(time), ..Default::default()
        }, |ui| {
            scene.begin_frame(ui, DiscoMode::Static, true, false, AudioReactive::default());
            let frame = scene.pending.as_ref().unwrap();
            let layer = frame.painter.layer_id();
            let index = frame.rays.0;
            scene.finish_frame(ctx);
            ctx.graphics(|graphics| {
                let entry = graphics.get(layer).unwrap().all_entries().nth(index).unwrap();
                if let Shape::Vec(shapes) = &entry.shape {
                    for shape in shapes {
                        if let Shape::Mesh(mesh) = shape {
                            result.extend(mesh.vertices.iter().map(|vertex| vertex.color.a()));
                        }
                    }
                }
            });
        });
        result
    }

    #[test]
    fn qa_static_rays_fade_instead_of_staying_at_constant_opacity() {
        let ctx = egui::Context::default();
        let mut scene = DiscoScene::new(&ctx, &[]);
        scene.random = 0x12345678;
        scene.set_ray_count(1);
        let first = ray_alphas(&mut scene, &ctx, 1.0);
        assert!(!first.is_empty());
        let samples: Vec<_> = (1..=120).map(|i| ray_alphas(&mut scene, &ctx, 1.0+i as f64/60.0)).collect();
        assert!(samples.iter().any(|alpha| alpha != &first), "Beam alpha is constant over 120 frames; only color jumps on deadlines");
    }

    #[test]
    fn qa_zero_static_rays_is_a_valid_setting_without_panic() {
        let ctx = egui::Context::default();
        let mut scene = DiscoScene::new(&ctx, &[]);
        scene.set_ray_count(0);
        crate::test_support::run_ui(&ctx, egui::RawInput { time:Some(1.0),..Default::default() }, |ui| {
            scene.begin_frame(ui, DiscoMode::Static, true, false, AudioReactive::default());
        });
    }

    #[test]
    fn qa_zero_moving_rays_removes_panel_reflections_as_documented() {
        let ctx = egui::Context::default();
        let mut scene = DiscoScene::new(&ctx, &[]);
        scene.set_ray_count(0);
        let output = crate::test_support::run_ui(&ctx, egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0,790.0))),
            time: Some(1.0), ..Default::default()
        }, |ui| {
            scene.begin_frame(ui, DiscoMode::Moving, true, false, AudioReactive::default());
            register_panel(&ctx, Rect::from_min_size(Pos2::new(50.0,50.0),Vec2::new(300.0,400.0)), 14.0);
            scene.finish_frame(&ctx);
        });
        let panel = Rect::from_min_size(Pos2::new(50.0,50.0),Vec2::new(300.0,400.0));
        let glows = output.shapes.iter().filter(|shape| {
            matches!(&shape.shape, Shape::Mesh(mesh) if !mesh.vertices.is_empty() && mesh.vertices.iter().all(|v| panel.contains(v.pos)) && mesh.vertices.iter().any(|v| v.color.a()>0))
        }).count();
        assert_eq!(glows, 0, "paint_wash runs for every panel even when ray_count=0");
    }
}
