using System;
using Test;

static void Register(TestService service)
{
    Func<string, string> handler = request => request;
    service.add_handler("/", handler);
}

_ = (Action<TestService>)Register;
