extends Node2D

# The Godot half of the lighting comparison in `docs/benchmarking.md` §18.
#
# It mirrors frost's `benchsuite` on purpose — same ladder of scenarios, same
# 1 s warm-up, same 6 s measurement, same three interleaved reps with the
# median reported — so the two tables can be read side by side. What it cannot
# mirror is the lighting model: Godot's 2D lights draw a light texture over a
# darkened canvas and build shadows by rendering occluder silhouettes into a
# mask, where frost traces rays per lit pixel. So the scenarios are named for
# the *work they ask for*, not claimed to be the same pixels: `shadows-4x` is
# Godot's softness ladder (MSAA on the shadow mask) standing in for frost's
# penumbra taps, and the mapping is stated in the document rather than hidden
# here.
#
# Geometry: `--fullscreen` puts the window in the same 4112x2572 Space frost's
# `--fullscreen` settles at, which is the only configuration on this platform
# where the frame rate is the machine's and not the panel's — measured, an
# empty scene reads 60.1 fps in a window and 216.7 fps in fullscreen. Godot
# draws at content scale 1.00, so "window pixels" here are physical pixels and
# both engines measure the same raster in that configuration.

const WARM_SECONDS := 1.0
const MEASURE_SECONDS := 6.0
const REPS := 3

# Shortened by `--quick`, which exists so the harness can be validated in
# twenty seconds instead of three minutes without editing constants.
var measure_seconds := MEASURE_SECONDS
var reps := REPS

# name, lights, occluders, msaa on the shadow mask.
const SCENARIOS := [
	{"name": "fill", "lights": 0, "occluders": 0, "msaa": Viewport.MSAA_DISABLED},
	{"name": "lamps-4", "lights": 4, "occluders": 0, "msaa": Viewport.MSAA_DISABLED},
	{"name": "lamps-10", "lights": 10, "occluders": 0, "msaa": Viewport.MSAA_DISABLED},
	{"name": "lamps-40", "lights": 40, "occluders": 0, "msaa": Viewport.MSAA_DISABLED},
	{"name": "occluders-10", "lights": 10, "occluders": 10, "msaa": Viewport.MSAA_DISABLED},
	{"name": "occluders-100", "lights": 10, "occluders": 100, "msaa": Viewport.MSAA_DISABLED},
	{"name": "shadows-1x", "lights": 10, "occluders": 60, "msaa": Viewport.MSAA_DISABLED},
	{"name": "shadows-4x", "lights": 10, "occluders": 60, "msaa": Viewport.MSAA_4X},
	{"name": "shadows-8x", "lights": 10, "occluders": 60, "msaa": Viewport.MSAA_8X},
]

var results: Array = []
var queue: Array = []
var world: Node2D
var light_texture: Texture2D
var fill_texture: Texture2D

var t := 0.0
var frames := 0
var warm_done := false


func _ready() -> void:
	DisplayServer.window_set_vsync_mode(DisplayServer.VSYNC_DISABLED)
	Engine.max_fps = 0
	var args := OS.get_cmdline_user_args()
	if "--quick" in args:
		measure_seconds = 2.0
		reps = 1
	if "--fullscreen" in args:
		get_window().mode = Window.MODE_FULLSCREEN
	if "--stretch-1080p" in args:
		# Not wired: Godot 4.7 moved the content-scale enums somewhere the
		# harness cannot name without guessing, and the matched comparison
		# this tool exists for is both engines at native fullscreen pixels.
		# frost's own answer to "what if the render buffer is 1080p" is
		# `benchsuite --render-size 1920x1080`, which is the half of the
		# question that can be answered on this machine today.
		push_warning("godot-compare: --stretch-1080p is not implemented; measuring at window pixels")
	print(
		(
			"godot-compare: %s, %s (engine version on the line above)"
			% [
				"fullscreen" if "--fullscreen" in args else "windowed",
				"canvas at window pixels",
			]
		)
	)
	print("godot-compare: adapter %s" % RenderingServer.get_video_adapter_name())
	world = Node2D.new()
	add_child(world)
	light_texture = _radial_texture(256)
	fill_texture = _flat_texture(64)
	# The darkened canvas is what makes a 2D light cost anything at all: with a
	# white canvas there is nothing to lighten.
	var darken := CanvasModulate.new()
	darken.color = Color(0.18, 0.18, 0.2)
	add_child(darken)
	# One second for the fullscreen transition to land before anything counts.
	await get_tree().create_timer(1.0).timeout
	var size := get_window().size
	var scale := DisplayServer.screen_get_scale()
	print("godot-compare: drew into %s window px at content scale %.2f, screen scale %.2f" % [
		size,
		get_window().content_scale_factor,
		scale,
	])
	for rep in reps:
		for s in SCENARIOS:
			queue.append(s)
	_start_scenario()


func _process(delta: float) -> void:
	t += delta
	if t < WARM_SECONDS:
		return
	if not warm_done:
		warm_done = true
		frames = 0
		t -= WARM_SECONDS
		return
	frames += 1
	if t >= measure_seconds:
		var fps := float(frames) / (t if t > 0.0 else 0.001)
		# Per scenario, not once at the end: a window macOS has parked behind
		# another is throttled to half the refresh rate, and a run like that
		# looks exactly like a slow machine — measured, an unfocused window
		# here reads 30.2 fps on a scene that costs nothing.
		print("godot-compare: %s %0.1f fps (can draw %s)" % [queue[0]["name"], fps, DisplayServer.window_can_draw()])
		results.append([queue[0]["name"], fps])
		queue.pop_front()
		_start_scenario()


func _start_scenario() -> void:
	if queue.is_empty():
		_finish()
		return
	for child in world.get_children():
		child.queue_free()
	world.get_children().clear()
	t = 0.0
	frames = 0
	warm_done = false
	get_viewport().msaa_2d = queue[0]["msaa"]
	var s: Dictionary = queue[0]
	var extent := _canvas_extent()
	# The lit field: enough flat sprites to give the lights real pixels to
	# cover, scattered by the same integer hash frost's suite uses so neither
	# scene is kinder to itself.
	for i in 240:
		var sprite := Sprite2D.new()
		sprite.texture = fill_texture
		sprite.scale = Vector2(6, 6)
		sprite.position = _scatter(i, extent)
		world.add_child(sprite)
	for i in int(s["lights"]):
		var light := PointLight2D.new()
		light.texture = light_texture
		# 4.7 has no `range` on PointLight2D: a point light is its texture, so
		# size is `texture_scale` over the 256 px radial generated below —
		# 3.3 puts the falloff at roughly the 420 px frost's lamps use.
		light.texture_scale = 3.3
		light.energy = 0.9
		light.shadow_enabled = int(s["occluders"]) > 0
		light.position = _scatter(i + 1000, extent)
		world.add_child(light)
	for i in int(s["occluders"]):
		var occluder := LightOccluder2D.new()
		var polygon := OccluderPolygon2D.new()
		var half := Vector2(90, 60)
		polygon.polygon = PackedVector2Array([-half, Vector2(half.x, -half.y), half, Vector2(-half.x, half.y)])
		polygon.closed = true
		occluder.occluder = polygon
		occluder.position = _scatter(i + 7919, extent)
		world.add_child(occluder)


func _finish() -> void:
	var medians := {}
	for s in SCENARIOS:
		var samples: Array = []
		for r in results:
			if r[0] == s["name"]:
				samples.append(r[1])
		samples.sort()
		medians[s["name"]] = samples[samples.size() / 2]
	for s in SCENARIOS:
		print("godot-compare: %s median %.1f fps = %.2f ms" % [s["name"], medians[s["name"]], 1000.0 / medians[s["name"]]])
	print("godot-compare: can draw %s" % DisplayServer.window_can_draw())
	get_tree().quit()


func _canvas_extent() -> Vector2:
	# Godot draws window pixels one to one at content scale 1.00, which is what
	# makes its "1080p" and frost's `--render-size 1920x1080` the same pixels.
	return Vector2(get_window().size)


func _scatter(i: int, extent: Vector2) -> Vector2:
	var x := float((i * 2654435761) % 10007) / 10007.0
	var y := float((i * 40503) % 9973) / 9973.0
	return Vector2(x * extent.x, y * extent.y)


func _flat_texture(side: int) -> Texture2D:
	var image := Image.create(side, side, false, Image.FORMAT_RGBA8)
	image.fill(Color(0.9, 0.85, 0.75))
	return ImageTexture.create_from_image(image)


func _radial_texture(side: int) -> Texture2D:
	var image := Image.create(side, side, false, Image.FORMAT_RGBA8)
	var centre := Vector2(side, side) / 2.0
	for y in side:
		for x in side:
			var d := Vector2(x, y).distance_to(centre) / centre.x
			var a := clampf(1.0 - d, 0.0, 1.0)
			image.set_pixel(x, y, Color(1, 1, 1, a * a))
	return ImageTexture.create_from_image(image)
