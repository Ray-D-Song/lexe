# Test runner

Test runner uses a lightweight Jest-like API and supports Jest/Chai assertions. For examples on how to implement tests for raster_runtime see the `/tests` folder of this repository.

To run tests, execute the `raster_runtime test` command. raster_runtime scans the current directory and sub-directories for files that ends with `*.test.js` or `*.test.mjs`. You can also provide a specific test directory to scan by using the `raster_runtime test -d <directory>` option.

The test runner also has support for filters. Using filters is as simple as adding additional command line arguments, i.e: `raster_runtime test crypto` will only run tests that match the filename containing `crypto`.
