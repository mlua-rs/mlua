local result1 = require("./dependency")
local result2 = require("./dependency")
assert(result1 == result2, "expect the same result when requiring the same module twice")
return {}