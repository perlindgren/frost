# General Todos

- /plan I want a Body type that carries velocity, weight and shape. When colliding with other bodies, their velocity should be altered by the normal of the collision such that no energy is lost. As an example place rectangles (rigid bodies) at the top, bottom, left and right, with a width of 50 pixels, and 3 rectangle shaped Bodies at random non overlapping positions at the center. Each of these bodies should have a random initial x and y velocity in between 50 and 100 pixels per second. The bodies should move according to their position and may collide against the walls. 

## Immortal Tomato

- Graphics
    - Shop Icon
    - Progression towards Immortality
    - Level indicator for water and fertilizer Grow Up

- Sound effects 
    - watering, spraying, ketchup blobs 
    - Tomatoes falling to ground
    - Bees swarming
    - Bugs walking and chatting
    - Tomatoes chatting
    - Bugs eliminated (bumped and falling over)

- Music 
    - menu music, game loop music

- Tomato Flower -> Tomatoes (ripe) -> Rotten -> Fall down

- Bees Swarming 

- Bugs attacking root of tree, bugs climbing 

## Concrete prompts

- /plan Now we want tomatoes to grow. Each plant is made from 5 layers.
The layers have the following spawn coordinates, relative to their local coordinate system.
 - plant1 (308,615) # root of plant
 - plant2 (306,496), (638,428), (304,392)
 - plant3 (315,392), (326,315), (297,258), (320,203), (300,128)
 - plant4 (339,364), (298,318), (356,258), (388,228), (334,203), (283,182)
 - plant5 # top of plant, no tomatoes spawned at this layer
When a layer is fully grown, it should start spawning tomatoes. For now we just want to ensure that the spawn points are correct.
Use the sprite `assets/sprites/flower.png` to render the tomato.
 