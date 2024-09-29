TODO
====

Features
--------

* add horizontal scrollbar to DRM clients block
* add driver backend to get specific dev info: frequency, dev type (integrated/discrete)
* use device type and mem regions to:
  * show overall GPU usage and smem vs vram/lmem usage
  * show GPU resident mem usage on smem vs vram/lmem per client
* add detailed view for a client
  * bar graphs for mem per region and for engines
* show cpu % usage and smem (tot/rss) usage, show gpu % usage and overall mem (tot/rss) per client (?)

Code Quality/Structure
----------------------

* improve code comments
* review
  * add more Option<> fields, store refs and not owned vals
  * visibility of fields and methods
