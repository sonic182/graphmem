<?php

namespace App;

class Report
{
    public function render(): string
    {
        return "report";
    }
}

function build_report(): Report
{
    return new Report();
}

require "vendor/autoload.php";
require_once "bootstrap.php";
include "helpers.php";
include_once "legacy.php";
