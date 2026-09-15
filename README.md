# Vis Viva

A real-time-with-pauses, event-driven space colony survival strategy game. Start off in a sleeper ship sent across the galaxy to a system _believed_ to have a habitable world, with finite starter resources.

## TODO:
- [x] Procedurally generated solar system
- [x] Delta-V
- [x] Stages/payloads
- [x] Dynamic UI elements
- [x] station
    - [x] station entity with modules/components (start off hovering this! no factory/vab!)
        x station cores have slots for modules, shown in the context menu
        x has builtin batteries
    - [x] life support: power, O2, water
        x Solar array as module payload slots, should probably start power-positive, but just barely
        x water tanks
        x H2 and O2 tanks, electrolysis module
    - [x] crew: consume O2 and water
    - [x] display time-to-zero (and time-to-fill)
    - [x] fabricator module: converts feedstock into parts
        x metal as a part
        x recipe data in the toml
        x affordability function, shortfalls()
        x modal shell with cards, read-only
        x build button
        x commits on Next Turn, not build
        x Energy as a continuous draw
        x gui_structure_key includes job state
        x replace build button with progress bar, "Done by ... " text
        x show inventory
    - [x] force there to be a jupiter-analog
- [ ] MVP stuff
    - [ ] better mission planning
        x tell me in the mission planner UI if the lambert solver failed
        x rendevouz maneuver
        x porkchop plot: departure window, tof window, initialize with planner's choice
        x add picker to porkchop plot
        x theta
        x don't offer "escape" if parent is the sun, etc
        x labels, axes, numbers on the porkchop plot
        x add departure time
        * either tooltip or desc paragraph of what each maneuver kind means
        * add critical events as diagonal lines on the porkchop plot
    - [x] better time controls, speed control
        x play becomes pause becomes play button
        x fast forward and slow forward, show speed, powers of two days/sec
    - [x] craft/body list
        x bread crumbs (Mars > Deimos > Deimos Station) (upwards traversal)
        x bodies have lists of other bodies and craft (downwards traversal)
    - [x] better descriptions for things and what you're supposed to do. What is a "Dray", what is a "Pico", why do I want either?
    - [ ] better events/timeline
        - [x] event descs with full details, take me to the entity (station) that's built something
            x resolve `craft: Entity` to the name of the craft
            x store `label: &'static str` on `Burn`, get it from `Command::burn_schedule()`
            x What part is complete? What were you building?
        - [x] event list on left side, shows what's next, what's paused
        - [x] scissor timeline so that dates dont draw off the side
        * make buttons/sliders pause automatically
        * gotta figure out a way to include the year in the event list!!
        * clicking event rows, or events on the timeline, takes you to the craft
    - [ ] better craft info
        * tell me the orbital elements for a craft/body
        x tell how long until burns
        x tell me what dv each burn costs
        x tell me in the craft's mission where its going! I forgot!
    - [x] better fabrication
        x list byproducts
        x gray out fully if a part is unbuildable
        x gray out if 0 in the inventory
        x mark some parts as un-fabricatable (ilmenite, station core) and don't list them
        x fabricator shows power draw rate, completion date, and days-until-completion
    - [x] make linepaths participate in occlusion again
    - [x] lose the game if the station dies
        * stop ahead of depletion (30 days, 7 days, 1 day)
    - [ ] basic mining
        * dray has water tank, mining module, battery
        * when on surface, can mine, fills water tank
        * miner draws power
        * changes the craft's weight!
    - [ ] docking
        * probably want another tab for docking ports, otherwise it gets crayzay
        * can dock if close enough, and a port is available
        * can undock
        * fabricating a craft takes up a docking spot, open the station's docking tab on completion
            - If we make this the same requirement for modules, modules could just take up slots, the same as craft
    - [ ] resource transfer
        * Dray and Pico have actual H2 and O2 tanks, taken from by engines
        * resource transfer modal
        * dont just take H2 and O2, transfer to craft
- [ ] Mining && ISRU
    - [x] Ilmetite smelting (just give generic "metal" for MVP)
    - [x] station rendevous
    - [ ] when on surface: able to mine ice and ilmenite, into cargo hold and water tank
        * copy `volatiles = sample_ice_fraction * tidal_ice_retention` into Body from solar_system_gen, use that for the Ice abundance
        * `crustal_metals = f_iron * (1 - core_mass_fraction)`
        * tile temp for bodies, colder near poles
        * terrain, low freq noise map over the tiles
        * ice = volatiles * tile-temp
        * ilmenite = crustal_metals * differentiation(radius) * mare_weight(terrain)
        * abundance = "total tonnage at that site"
            - largely uncertain, with high noise floor
        * availability = "energy per kg"
            - cheaply inferable, no noise floor, otherwise players get mad. Gambles are **ALWAYS** on reward, never on cost. Never hestiate to randomly reward the player. Never punish the player for something they have no control of/insight into.
            * for ice: a function of the tile_map and the `rough` param
            * for ilmenite: a function of atmospheric pressure and surface gravity and the `rough` param
    - [ ] inventory transfer modal
    - [ ] ability to choose your landing site from what's available underneathe you
    - [ ] surface outpots on tiles (give them solar panels for now)
    - Rules: 
        * Every resource should have a useful role somewhere in the system, and preferably a secondary use that competes with its first.
        * The byproducts of processes are always useful.
        * Natural resources are clustered out in the system, imperfectly overlap, and distributed based on the hidden parameters.
    - Eventual modules:
        x Electrolysis: H2O + Energy -> H2 + O2
        * Hydrolox Fuel Cell: H2 + O2 -> H2O + Energy (cleaner, but H2 tanks should be a pain)
        * Methalox Fuel Cell: CH4 + O2 -> H2O + CO2 + Energy (not as nice with the CO2, but no H2)
        * Chemistry Lab: Has cartridges for specific processes:
            * Sabatier: CO2 + H2 -> CH4 + H2O
            * Methane Pyrolysis: CH4 -> C, H2
            * CO2 Scrubbing: CaO + CO2 -> CaCO3
            * Haber Bosch: N2 + H2 -> NH3
        * Smelter does a bunch of refinements:
            * TiO2 + Energy -> Ti O2
            * Al2O3 + Energy -> Al + O2
            * SiO2 + Energy -> Si + O2
            * CaCO3 + Energy -> CaO + CO2
        * Greenhouse: CO2 + H2O + Energy -> Food + O2 (composes maybe too well with methalox fuel cell?)
- [ ] Science
    - [ ] body rotation, axial tilt
        - allows polar mapping probes to actually exist
        * axial tilt affects seasons, temps, could have crazy uranus worlds
        * rotation affects landing and launch delta V
    - [ ] measurements
        - z = W * X * H^T + epsilon(X)
            - z is the measurement vector
            - W is the light-of-sight footprint weights over tiles
            - X is N_tiles x N_params truth state
            - H^T is the instrument-sensitivity
            - epsilon(X) is state-dependent noise
        - D += (WH)^T R^-1 (WH)
        - nu += (WH)^T R-1 z
    - [ ] parameters
        - [T_surf, ice, maf, depth, rough]
            - T_surf: surface pressure, ice abundance (smooth, from tile temp map)
            - ice: water ice mass fraction
            - maf: mafic rock mass fraction
            - depth: burial depth (smooth, from tile terrain perlin)
            - rough: topographic roughness
    - [ ] H matrix
        |                      | T_surf | ice | maf | depth | rough |
        |----------------------|--------|-----|-----|-------|-------|
        | IR Spectrometer      |    0.1 | 0.9 | 0.6 |       |       |
        | IR Radiometer        |    1.0 |     |     |   0.3 |   0.2 | Perhaps add a day/night pair?
        | Neutron Spectrometer |        | 1.0 |     |   0.3 |       |
        | Radar                |        | 0.5 | 0.2 |   0.4 |   0.9 | ambigous on purpose!
    - [ ] Noise
        * IR Spectrometer returns nothing on an unlit tile
        * IR Spectrometer noise scales with depth
        * Optics noise scales with phase angle and slant range
    - [ ] W
        * Classic tradeoff of resolution vs coverage, flybys want full coverage, orbits want low eventual resoltuion
    - [ ] Add spatial smoothness prior
    - [ ] Show mean + sigma, never truth. Tile coloring overlays by posterior mean, saturated with confidence
- [ ] Game saves and loading
- [ ] Misc stuff
    - [ ] incr craft names, or let people rename them
    - [ ] events should give countdown and absolute datetime
    - [ ] timeline zoom (maybe by dragging the baseline?)
    - [ ] ability to ignore/subscribe to events (like infos)
    - [ ] combined mission planner
        * Players create a mission by sequencing maneuvers together
        * Can tweak each one to optimize the total mission, not just each maneuver
    - [ ] gravity assist planner
        * tisserand plots? Or just show me the energy gained/lost for each flyby
    - [ ] expose target periapsis, once it matters to the player (when they build their own depot stations)
    - [ ] allow aero capture when payload has heat shield and player has atmospheric instrument readings
    - [ ] able to build other stations
        * game is only over when _all_ crew are dead, not just the starter station
    - [ ] module swapping and building system for stations
        * stations should be able to build new modules in open slots, jettison existing ones
    - [ ] heat management
        * radiators would be weaker near the sun
    - [ ] atmospheric harvesting
        * get H2 from gas giants, NH3 from ice giants, CO2 from venus-worlds
    - [ ] boiloff for cryo fuels, cooling systems which add weight and take power, tradeoff between cryogenics and hypergolic fuels.
    - [ ] different power sources
        * solar: inverse square proximity to sun => (energy)
        * hydolox fuel cell: H2 + O2 => (energy) + H2O
        * methalox fuel cell: CH4 + 2 O2 => (energy) + C02 + 2 H2O
        * nuclear: U234 => (energy) (heavy but materially efficient)
    - [ ] crew respiration produces small amounts of CO2 that needs to be scrubbed. Later on can be captured.
    - [ ] crew capacity, determine how many simultaneous things can go on in the station.
    - [ ] radiation as a hazard to shield against
        * crew die if they're exposed to radiation, shielding adds mass, mass affects delta V
    - [ ] craft need parachute/landing gear in order to land
    - [ ] win if you beam a message back to earth, huge amount of power, megaproject
- [ ] Misc polish
    - [ ] planetary atmospheres, clouds, tile detail
    - [ ] better orbit icons, for zoomed out moons, planets, craft. Somehow distinguish between bodies and craft in the orbit view
    - [ ] tooltips, when I figure out what tools to tip
        * manuever kinds in the dropdown
        * what modules do, when you hover over their title
        * destinations in the planner, its size, its composition, etc
    - [ ] let people explore the game after game over, but disable the play button
    - [ ] make actual building models, and rotate them to their tile's normal
    - [ ] show more info about stage cards, their dv, their resources, maybe a little model sprite, in VAB and factory
    - [ ] somehow show the planned orbital geometry to a player, if it ends up mattering (tuning just dv and time might be fine?)
    - [ ] show target craft/bodies whenever we have a rendezvous/transfer/flyby planned
        - maybe like a little tooltip by the target craft/body that says "RENDEZVOUS: 17 days"
    - [ ] show the planned trajectory for crafts with a mission, different color, lighter
    - [x] show how many stages we have in inventory in the factory
    - [ ] "kernel boot screen" type loading screen, TUI-esque main menu, like you're interfacing with the "Autonomous Colony Management System" that the player is for the game
        * A skippable intro scene when you start a new game that tells you that you're the AI controlling the station, the backstory, what the goals are
    - [ ] (purely code) strongly typed units. Dont gotta go full mp-units crazy, and it probably wouldn't work too well with nalgebra... but yknow