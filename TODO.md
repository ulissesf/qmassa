TODO
====

Features
--------

* [Ulisses WIP] improve/find mainloop to support different resolution timers
* [Ulisses WIP] option to create png files for different plotted graphs (great idea from Rodrigo Vivi)
* refactor UI code to support multiple screens
  * simplify and consolidate UI styles being used
  * add detailed info screen for clients
  * show graphs for mem per region and for engines
* get power limits per driver
  * use limits in power chart
* add driver feature support flags so the app UI knows what data to use/render
  * probably another method in DrmDriver trait

Code Quality/Structure
----------------------

* reduce usage of owned Strings
* improve code comments
* provide both lib and app crates
