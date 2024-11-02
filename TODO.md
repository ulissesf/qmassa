TODO
====

Features
--------

* [Ulisses WIP] improve/find mainloop to support different resolution timers
* [Ulisses WIP] option to create png files for different plotted graphs (great idea from Rodrigo Vivi)
* show device power usage for igfx and dgfx (hwmon)
  * [Ulisses WIP] get power levels for igfx from MSR or perf
  * [Rodrigo Vivi WIP] expose generic HwMon interface to read the data
  * integrate with app data layer and UI
* add driver feature support flags so the app UI knows what data to use/render
  * probably another method in DrmDriver trait
* option to select which device stat graphs to plot
  * maybe add scroll view also in device area (and handle focus between areas)
* refactor UI code to support multiple screens
  * simplify and consolidate UI styles being used
  * add detailed info screen for clients
  * show graphs for mem per region and for engines

Code Quality/Structure
----------------------

* reduce usage of owned Strings
* improve code comments
* provide both lib and app crates
